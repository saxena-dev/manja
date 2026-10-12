//! Dispatch: a mutation's attempt runs to completion whatever the caller
//! does with its future, and its outcome reaches the outcome stream or the
//! orphan ring exactly once. Against the loopback harness, with a reply
//! delayed long enough to drop the future after the request was sent.

mod support;

use std::time::Duration;

use manja::kite::connect::admission::{
    Admission, AdmissionLimits, QuotaProfile, RateClass, Window,
};
use manja::kite::connect::client::{HTTPClient, HttpClientBuilder};
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::dispatch::{
    Dispatched, ORPHANED_OUTCOMES, Outcome, OutcomeStream, OutcomeStreamError,
};
use manja::kite::connect::models::{
    Exchange, OrderReceipt, OrderType, OrderVariety, PlaceOrderRequest, ProductType,
    TransactionType,
};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpErrorKind, TransportStage};
use manja::kite::obs::schema::{Endpoint, HttpOperationResult, Method};
use manja::kite::protocol::{OrderId, Quantity};

use support::fixtures;
use support::http::{HttpHarness, Reply};

fn market_buy() -> PlaceOrderRequest {
    PlaceOrderRequest::new(
        OrderVariety::Regular,
        Exchange::NSE,
        "INFY",
        TransactionType::BUY,
        OrderType::Market,
        Quantity::new(1).unwrap(),
        ProductType::CashAndCarry,
    )
}

fn receipt() -> Vec<u8> {
    fixtures::json_body("order_response.json")
        .unwrap()
        .into_bytes()
}

fn delayed(after: Duration) -> Reply {
    Reply::Delayed {
        after,
        body: receipt(),
    }
}

fn builder(base: &str) -> HttpClientBuilder {
    // A fast placement window: these tests are about dispatch, not quota.
    let profile = QuotaProfile::kite_v3()
        .with_version("dispatch")
        .with_windows(
            RateClass::OrderPlacement,
            vec![Window::new(10, Duration::from_secs(1))],
        )
        .unwrap();
    HTTPClient::builder(Config::new(base))
        .admission(Admission::new(profile, AdmissionLimits::default()))
        .credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

fn with_stream(base: &str) -> (HTTPClient, OutcomeStream) {
    builder(base).build_with_outcomes().unwrap()
}

/// Wait until the harness has recorded `n` requests: the attempts are past
/// `Started`.
async fn sent(h: &HttpHarness, n: usize) {
    for _ in 0..200 {
        if h.requests().len() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{n} request(s) never reached the harness");
}

#[tokio::test]
async fn a_dropped_dispatch_still_delivers_its_outcome_to_the_stream_once() {
    let h = HttpHarness::start(vec![delayed(Duration::from_millis(300))]).await;
    let (c, mut outcomes) = with_stream(&h.base_url());
    let c = c.with_correlation("attempt-42").unwrap();
    let request = market_buy();
    let wire = request.wire().unwrap();
    let dispatched: Dispatched<OrderReceipt> = c.orders().place_order(&request);
    assert!(dispatched.refusal().is_none());
    assert!(dispatched.is_in_flight());
    assert_eq!(dispatched.wire_sha256(), Some(wire.sha256()));
    assert_eq!(dispatched.correlation(), Some("attempt-42"));
    let operation_id = dispatched.operation_id();
    // The request is on the wire: drop the future.
    sent(&h, 1).await;
    drop(dispatched);
    // Exactly one outcome, with everything the engine journals.
    let outcome = tokio::time::timeout(Duration::from_secs(5), outcomes.recv())
        .await
        .expect("an outcome arrives")
        .expect("the stream is open")
        .expect("a delivered outcome");
    assert_eq!(outcome.operation_id, operation_id);
    assert_eq!(outcome.correlation.as_deref(), Some("attempt-42"));
    assert_eq!(outcome.wire_sha256, wire.sha256());
    assert_eq!(
        (outcome.method, outcome.endpoint),
        (Method::Post, Endpoint::OrdersVariety)
    );
    assert_eq!(outcome.stage, TransportStage::ResponseReceived);
    assert_eq!(outcome.result, HttpOperationResult::Ok);
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.http_status, Some(200));
    assert_eq!(outcome.attempt, 1);
    assert_eq!(
        outcome.receipt.order_id,
        Some(OrderId::new("151220000000000").unwrap())
    );
    assert_eq!(outcome.receipt.trigger_id, None);
    assert_eq!(&outcome.source, c.source_identity());
    let (sent_at, received_at) = (outcome.sent_at.unwrap(), outcome.received_at.unwrap());
    assert!(received_at.monotonic >= sent_at.monotonic);
    assert!(received_at.wall.unix_nanos() - sent_at.wall.unix_nanos() >= 250_000_000);
    // Nothing more, and no cancellation anywhere.
    assert!(outcomes.is_empty());
    let d = c.diagnostics();
    assert!(
        d.last_failures
            .iter()
            .all(|f| f.kind != HttpErrorKind::Cancelled),
        "{:?}",
        d.last_failures
    );
    assert_eq!(d.dispatches_in_flight, 0);
    assert_eq!(d.orphaned_outcome_count, 0);
    assert_eq!(d.outcome_stream_dropped, 0);
    assert_eq!(h.requests().len(), 1);
}

#[tokio::test]
async fn without_a_stream_a_dropped_dispatch_lands_in_the_orphan_ring() {
    let h = HttpHarness::start(vec![
        delayed(Duration::from_millis(200)),
        delayed(Duration::ZERO),
    ])
    .await;
    let c = builder(&h.base_url()).build().unwrap();
    let dispatched = c.orders().place_order(&market_buy());
    let operation_id = dispatched.operation_id();
    sent(&h, 1).await;
    drop(dispatched);
    for _ in 0..200 {
        if c.diagnostics().dispatches_in_flight == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let d = c.diagnostics();
    assert_eq!(d.orphaned_outcome_count, 1);
    assert_eq!(d.orphaned_outcomes.len(), 1);
    assert_eq!(d.orphaned_outcomes[0].operation_id, operation_id);
    assert_eq!(d.orphaned_outcomes[0].result, HttpOperationResult::Ok);
    assert_eq!(d.outcome_stream_dropped, 0);
    // An awaited dispatch is not an orphan: the ring is unchanged.
    c.orders().place_order(&market_buy()).await.unwrap();
    let d = c.diagnostics();
    assert_eq!(d.orphaned_outcome_count, 1);
    assert_eq!(d.orphaned_outcomes.len(), 1);
    assert!(d.orphaned_outcomes.len() <= ORPHANED_OUTCOMES);
}

#[tokio::test]
async fn a_stalled_consumer_ends_the_stream_with_an_error_and_the_attempts_still_complete() {
    const CAPACITY: usize = 16;
    const DISPATCHES: usize = 20;
    let h = HttpHarness::start((0..DISPATCHES).map(|_| delayed(Duration::ZERO)).collect()).await;
    let profile = QuotaProfile::kite_v3();
    let (c, mut outcomes) = HTTPClient::builder(
        Config::new(h.base_url()).with_limits(
            HttpLimits::default()
                .with_outcome_capacity(CAPACITY)
                .unwrap(),
        ),
    )
    .admission(Admission::new(profile, AdmissionLimits::default()))
    .credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
    .build_with_outcomes()
    .unwrap();
    // Twenty dispatches, awaited one by one so each completes; the consumer
    // reads nothing until all are done. Placement admits ten a second, so
    // the second ten wait for the next second.
    for i in 0..DISPATCHES {
        if i > 0 && i % 10 == 0 {
            tokio::time::sleep(Duration::from_millis(1100)).await;
        }
        c.orders().place_order(&market_buy()).await.unwrap();
    }
    assert_eq!(h.requests().len(), DISPATCHES, "every attempt completed");
    // The stream yields its capacity, then one error naming the rest, then
    // ends; the rest sit in the ring, counted.
    let mut delivered = 0;
    let mut ended = None;
    while let Some(item) = outcomes.recv().await {
        match item {
            Ok(_) => delivered += 1,
            Err(e) => {
                ended = Some(e);
                break;
            }
        }
    }
    assert_eq!(delivered, CAPACITY);
    assert_eq!(
        ended,
        Some(OutcomeStreamError::Overflow {
            dropped: (DISPATCHES - CAPACITY) as u64
        })
    );
    assert!(outcomes.recv().await.is_none(), "the stream ended");
    let d = c.diagnostics();
    assert_eq!(d.outcome_stream_dropped, (DISPATCHES - CAPACITY) as u64);
    assert_eq!(d.orphaned_outcomes.len(), DISPATCHES - CAPACITY);
    assert_eq!(
        d.orphaned_outcome_count, 0,
        "not orphans: the stream was full"
    );
}

#[tokio::test]
async fn close_waits_for_the_attempt_in_flight_and_a_refusal_emits_no_outcome() {
    let h = HttpHarness::start(vec![delayed(Duration::from_millis(300))]).await;
    let (c, mut outcomes) = with_stream(&h.base_url());
    let dispatched = c.orders().place_order(&market_buy());
    sent(&h, 1).await;
    drop(dispatched);
    assert_eq!(c.diagnostics().dispatches_in_flight, 1);
    // A close that outlasts its deadline reports what is still in flight.
    let early = c.close(Duration::from_millis(20)).await.unwrap_err();
    assert_eq!(early.in_flight, 1);
    // A close with time returns only once the attempt delivered.
    let start = std::time::Instant::now();
    c.close(Duration::from_secs(5)).await.unwrap();
    assert!(
        start.elapsed() >= Duration::from_millis(150),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(c.diagnostics().dispatches_in_flight, 0);
    assert!(outcomes.recv().await.unwrap().is_ok());
    // A closing client refuses new dispatches; the refusal resolves at
    // once with itself and emits no outcome.
    let refused = c.orders().place_order(&market_buy());
    let refusal = refused.refusal().expect("refused before transport");
    assert_eq!(refusal.kind(), HttpErrorKind::Admission);
    assert_eq!(refusal.stage(), TransportStage::NotStarted);
    assert!(refused.wire_sha256().is_some());
    let err = refused.await.unwrap_err();
    assert_eq!(err.kind(), HttpErrorKind::Admission);
    assert!(outcomes.is_empty(), "a refusal is not an outcome");
    assert_eq!(h.requests().len(), 1);
}

#[tokio::test]
async fn a_full_window_refuses_the_dispatch_at_once_with_no_outcome_record() {
    let h = HttpHarness::start(vec![delayed(Duration::ZERO)]).await;
    let profile = QuotaProfile::kite_v3()
        .with_version("one-placement")
        .with_windows(
            RateClass::OrderPlacement,
            vec![Window::new(1, Duration::from_secs(60))],
        )
        .unwrap();
    let (c, mut outcomes) = HTTPClient::builder(Config::new(h.base_url()))
        .admission(Admission::new(profile, AdmissionLimits::default()))
        .credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
        .build_with_outcomes()
        .unwrap();
    c.orders().place_order(&market_buy()).await.unwrap();
    assert!(outcomes.recv().await.unwrap().is_ok());
    let refused = c.orders().place_order(&market_buy());
    let refusal = refused.refusal().unwrap();
    assert_eq!(refusal.kind(), HttpErrorKind::Admission);
    assert!(refusal.retry_after().is_some());
    let err = refused.await.unwrap_err();
    assert!(err.retry_after().is_some());
    assert!(outcomes.is_empty());
    assert_eq!(h.requests().len(), 1);
}

#[tokio::test]
async fn dispatches_beyond_the_in_flight_bound_are_refused_not_spawned() {
    let h = HttpHarness::start(vec![Reply::Stall, Reply::Stall, Reply::Stall]).await;
    let c = HTTPClient::builder(
        Config::new(h.base_url()).with_limits(
            HttpLimits::default()
                .with_in_flight(2)
                .unwrap()
                .with_scheduler(
                    SchedulerLimits::default()
                        .with_attempt_timeout(Duration::from_millis(200))
                        .unwrap()
                        .with_operation_deadline(Duration::from_secs(1))
                        .unwrap(),
                ),
        ),
    )
    .admission(Admission::new(
        QuotaProfile::kite_v3()
            .with_version("dispatch")
            .with_windows(
                RateClass::OrderPlacement,
                vec![Window::new(10, Duration::from_secs(1))],
            )
            .unwrap(),
        AdmissionLimits::default(),
    ))
    .credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
    .build()
    .unwrap();
    let first = c.orders().place_order(&market_buy());
    let second = c.orders().place_order(&market_buy());
    assert!(first.is_in_flight() && second.is_in_flight());
    sent(&h, 2).await;
    let third = c.orders().place_order(&market_buy());
    let refusal = third.refusal().expect("the third is refused");
    assert_eq!(refusal.kind(), HttpErrorKind::Admission);
    assert_eq!(refusal.stage(), TransportStage::NotStarted);
    assert!(refusal.detail().unwrap().as_str().contains("B-HTTP-10"));
    assert_eq!(
        c.diagnostics().dispatches_in_flight,
        2,
        "nothing was spawned"
    );
    assert_eq!(h.requests().len(), 2);
    // The two stalled attempts end with their attempt timeout, stage Started.
    let e = first.await.unwrap_err();
    assert_eq!(e.stage(), TransportStage::Started);
    assert!(e.is_timeout());
    second.await.unwrap_err();
}

#[tokio::test]
async fn a_correlation_token_is_bounded_and_a_dispatch_needs_a_runtime() {
    let h = HttpHarness::start(vec![delayed(Duration::ZERO)]).await;
    let (c, mut outcomes) = with_stream(&h.base_url());
    let too_long = "x".repeat(65);
    let err = c.with_correlation(too_long).unwrap_err();
    assert_eq!(err.as_http().unwrap().kind(), HttpErrorKind::Validation);
    assert!(c.with_correlation("").is_err());
    assert!(c.with_correlation("tab\tbed").is_err());
    // No call was made: nothing is recorded for a rejected token.
    assert!(c.diagnostics().last_failures.is_empty());
    let token = "y".repeat(64);
    let marked = c.with_correlation(token.clone()).unwrap();
    let dispatched = marked.orders().place_order(&market_buy());
    assert_eq!(dispatched.correlation(), Some(token.as_str()));
    dispatched.await.unwrap();
    let outcome: Outcome = outcomes.recv().await.unwrap().unwrap();
    assert_eq!(outcome.correlation, Some(token));
    // Outside a runtime the call is refused, not a panic.
    let refused = std::thread::spawn(move || {
        let d = marked.orders().place_order(&market_buy());
        (d.refusal().map(|e| e.kind()), d.is_in_flight())
    })
    .join()
    .unwrap();
    assert_eq!(refused, (Some(HttpErrorKind::Configuration), false));
}

#[test]
fn a_dispatched_future_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Dispatched<OrderReceipt>>();
    assert_send::<OutcomeStream>();
}

#[tokio::test]
async fn an_outcome_delivered_to_a_future_dropped_unpolled_is_still_kept() {
    // The attempt completes while the caller holds the future without
    // polling it; dropping it then hands the outcome to the ring, and with
    // a stream registered the stream already has it, so nothing is doubled.
    let h = HttpHarness::start(vec![delayed(Duration::ZERO)]).await;
    let c = builder(&h.base_url()).build().unwrap();
    let dispatched = c.orders().place_order(&market_buy());
    let operation_id = dispatched.operation_id();
    for _ in 0..200 {
        if c.diagnostics().dispatches_in_flight == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(c.diagnostics().orphaned_outcome_count, 0, "delivered, held");
    drop(dispatched);
    let d = c.diagnostics();
    assert_eq!(d.orphaned_outcome_count, 1);
    assert_eq!(d.orphaned_outcomes.len(), 1);
    assert_eq!(d.orphaned_outcomes[0].operation_id, operation_id);
    assert_eq!(d.orphaned_outcomes[0].result, HttpOperationResult::Ok);

    let h = HttpHarness::start(vec![delayed(Duration::ZERO)]).await;
    let (c, mut outcomes) = with_stream(&h.base_url());
    let dispatched = c.orders().place_order(&market_buy());
    for _ in 0..200 {
        if c.diagnostics().dispatches_in_flight == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(dispatched);
    let outcome = outcomes.recv().await.unwrap().unwrap();
    assert_eq!(outcome.result, HttpOperationResult::Ok);
    assert!(outcomes.is_empty());
    let d = c.diagnostics();
    assert_eq!(d.orphaned_outcome_count, 0);
    assert!(d.orphaned_outcomes.is_empty());
}
