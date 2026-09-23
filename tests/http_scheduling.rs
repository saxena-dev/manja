//! Deadlines, retries, single-attempt operations and cancellation evidence
//! (plan task S06), against a counting loopback server.
//!
//! Error bodies are labelled supplemental fixtures in the documented error
//! envelope shape (`kite-api-docs/docs/connect/v3/response-structure.md:17-28`).

mod support;

use std::time::{Duration, Instant};

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::{
    AccessToken, ApiKey, ApiSecret, Credentials, RequestToken,
};
use manja::kite::connect::models::{
    Exchange, ModifyOrderRequest, OrderType, OrderVariety, PlaceOrderRequest,
    PositionConversionRequest, PositionType, ProductType, TransactionType,
};
use manja::kite::connect::scheduler::{PermitTarget, SchedulerLimits};
use manja::kite::error::{HttpErrorKind, TransportStage};
use manja::kite::protocol::Quantity;

use support::fixtures;
use support::http::{HttpHarness, Reply};

fn client_with(base: &str, scheduler: SchedulerLimits) -> HTTPClient {
    let config = Config::new(base)
        .with_limits(HttpLimits::default().with_scheduler(scheduler.with_jitter_seed(3)));
    HTTPClient::with_config(config)
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

fn client(base: &str) -> HTTPClient {
    client_with(base, SchedulerLimits::default())
}

fn rate_limited() -> Reply {
    Reply::Respond {
        status: 429,
        content_type: "application/json",
        body:
            br#"{"status":"error","message":"Too many requests","error_type":"NetworkException"}"#
                .to_vec(),
    }
}

fn gateway(status: u16) -> Reply {
    Reply::Respond {
        status,
        content_type: "text/html",
        body: b"<html>gateway</html>".to_vec(),
    }
}

fn place_request() -> PlaceOrderRequest {
    let mut r = PlaceOrderRequest::new(
        OrderVariety::Regular,
        Exchange::NSE,
        "INFY",
        TransactionType::BUY,
        OrderType::Limit,
        Quantity::new(1).unwrap(),
        ProductType::CashAndCarry,
    );
    r.price = Some(1500.0);
    r
}

fn modify_request() -> ModifyOrderRequest {
    ModifyOrderRequest {
        price: Some(1501.0),
        ..Default::default()
    }
}

fn conversion() -> PositionConversionRequest {
    PositionConversionRequest {
        tradingsymbol: "INFY".into(),
        exchange: Exchange::NSE,
        transaction_type: TransactionType::BUY,
        position_type: PositionType::Overnight,
        quantity: Quantity::new(1).unwrap(),
        old_product: ProductType::CashAndCarry,
        new_product: ProductType::MarginIntradaySquareoff,
    }
}

/// Run every one-attempt operation against `replies` and assert that
/// exactly one request reached the server.
async fn each_one_attempt_operation(replies: fn() -> Vec<Reply>) {
    macro_rules! once {
        ($name:literal, |$c:ident| $call:expr) => {{
            let harness = HttpHarness::start(replies()).await;
            #[allow(unused_mut)]
            let mut $c = client(&harness.base_url());
            let result = $call.await;
            assert!(result.is_err(), "{}", $name);
            assert_eq!(harness.requests().len(), 1, "{} made one attempt", $name);
        }};
    }
    once!("place", |c| c.orders().place_order(&place_request()));
    once!("modify", |c| c.orders().modify_order(
        OrderVariety::Regular,
        "151220000000000",
        &modify_request()
    ));
    once!("cancel", |c| c
        .orders()
        .cancel_order(OrderVariety::Regular, "151220000000000"));
    once!("convert", |c| {
        let c2 = c.clone();
        async move {
            manja::kite::connect::api::Portfolio::new(&c2)
                .convert_position(&conversion())
                .await
        }
    });
    once!("exchange", |c| c
        .session(ApiKey::new("test_api_key").unwrap())
        .exchange(
            &RequestToken::new("request_token").unwrap(),
            &ApiSecret::new("test_api_secret").unwrap()
        ));
    once!("invalidate", |c| c
        .session(ApiKey::new("test_api_key").unwrap())
        .invalidate(&AccessToken::new("test_access_token").unwrap()));
}

#[tokio::test]
async fn mutations_and_session_operations_are_never_retried_after_429() {
    each_one_attempt_operation(|| vec![rate_limited(), rate_limited(), rate_limited()]).await;
}

#[tokio::test]
async fn mutations_and_session_operations_are_never_retried_after_response_loss() {
    each_one_attempt_operation(|| {
        vec![
            Reply::DropAfterRequest,
            Reply::DropAfterRequest,
            Reply::DropAfterRequest,
        ]
    })
    .await;
}

#[tokio::test]
async fn reads_retry_documented_transient_faults_then_succeed() {
    let harness = HttpHarness::start(vec![
        rate_limited(),
        gateway(502),
        Reply::json(fixtures::json_body("profile.json").unwrap()),
    ])
    .await;
    let fast = SchedulerLimits::default()
        .with_backoff(Duration::from_millis(10), Duration::from_millis(20))
        .unwrap();
    client_with(&harness.base_url(), fast)
        .user()
        .profile()
        .await
        .unwrap();
    assert_eq!(harness.requests().len(), 3);
}

#[tokio::test]
async fn read_retries_are_bounded_and_report_retry_metadata() {
    let harness =
        HttpHarness::start(vec![gateway(504), gateway(503), gateway(502), gateway(502)]).await;
    let fast = SchedulerLimits::default()
        .with_backoff(Duration::from_millis(10), Duration::from_millis(20))
        .unwrap();
    let err = client_with(&harness.base_url(), fast)
        .user()
        .profile()
        .await
        .unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(harness.requests().len(), 3, "B-HTTP-03 default is 3");
    assert_eq!(e.retry().unwrap().attempts, 3);
    assert_eq!(e.http_status(), Some(502));
}

#[tokio::test]
async fn a_500_is_not_retried() {
    let harness = HttpHarness::start(vec![gateway(500), gateway(500)]).await;
    let err = client(&harness.base_url())
        .user()
        .profile()
        .await
        .unwrap_err();
    assert_eq!(err.as_http().unwrap().http_status(), Some(500));
    assert_eq!(harness.requests().len(), 1);
}

#[tokio::test]
async fn the_operation_deadline_bounds_stalled_retries() {
    let harness = HttpHarness::start((0..5).map(|_| Reply::Stall).collect()).await;
    // Unbounded, five stalled 800 ms attempts would take about 4 s; the 1 s
    // deadline leaves room for at most two.
    let tight = SchedulerLimits::default()
        .with_attempt_timeout(Duration::from_millis(800))
        .unwrap()
        .with_operation_deadline(Duration::from_secs(1))
        .unwrap()
        .with_read_attempts(5)
        .unwrap()
        .with_backoff(Duration::from_millis(10), Duration::from_millis(20))
        .unwrap();
    // Build the client first: constructing a transport can be slow on a
    // loaded host, and the deadline bounds the operation, not construction.
    let client = client_with(&harness.base_url(), tight);
    let start = Instant::now();
    let err = client.user().profile().await.unwrap_err();
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(2500), "{elapsed:?}");
    assert!(
        harness.requests().len() <= 2,
        "{}",
        harness.requests().len()
    );
    let e = err.as_http().unwrap();
    assert!(e.is_timeout());
    assert!(e.may_have_reached_broker());
}

#[tokio::test]
async fn an_unpolled_dropped_future_sends_nothing() {
    let harness = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("profile.json").unwrap(),
    )])
    .await;
    let c = client(&harness.base_url());
    let user = c.user();
    let future = user.profile();
    drop(future);
    tokio::task::yield_now().await;
    assert!(harness.requests().is_empty());
    assert!(c.diagnostics().last_failures.is_empty());
}

#[tokio::test]
async fn cancellation_after_dispatch_is_recorded_as_started() {
    let harness = HttpHarness::start(vec![Reply::Stall]).await;
    let c = client(&harness.base_url());
    let cancelled = tokio::time::timeout(Duration::from_millis(200), c.user().profile()).await;
    assert!(
        cancelled.is_err(),
        "the stalled call was cancelled by the timeout"
    );
    assert_eq!(harness.requests().len(), 1);
    let d = c.diagnostics();
    let last = d.last_failures.last().unwrap();
    assert_eq!(last.kind, HttpErrorKind::Cancelled);
    assert_eq!(
        last.stage,
        TransportStage::Started,
        "no remote cancellation is claimed"
    );
    assert_eq!(d.active_attempts, 0);
}

#[tokio::test]
async fn cancellation_during_admission_is_recorded_as_not_started() {
    let harness = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("ltp.json").unwrap()),
        Reply::json(fixtures::json_body("ltp.json").unwrap()),
    ])
    .await;
    let c = client(&harness.base_url());
    let query = ["NSE:INFY"];
    c.market()
        .get_quotes::<manja::kite::connect::models::LTPQuote>(&query)
        .await
        .unwrap();
    // The quote class admits one per second: this call waits for admission
    // and is cancelled before any dispatch.
    let market = c.market();
    let waiting = market.get_quotes::<manja::kite::connect::models::LTPQuote>(&query);
    let cancelled = tokio::time::timeout(Duration::from_millis(100), waiting).await;
    assert!(cancelled.is_err());
    assert_eq!(
        harness.requests().len(),
        1,
        "the cancelled call sent nothing"
    );
    let last = c.diagnostics().last_failures.last().cloned().unwrap();
    assert_eq!(last.kind, HttpErrorKind::Cancelled);
    assert_eq!(last.stage, TransportStage::NotStarted);
    assert_eq!(c.admission().waiters(), 0);
}

#[tokio::test]
async fn permits_are_issued_by_the_clients_scope() {
    let harness = HttpHarness::start(vec![]).await;
    let c = client(&harness.base_url());
    let permit = c.admit(PermitTarget::PlaceOrder).await.unwrap();
    assert!(permit.belongs_to(c.admission()));
    assert!(!permit.is_expired());
    assert!(permit.remaining() <= Duration::from_secs(1));
    let other = client(&harness.base_url());
    assert!(!permit.belongs_to(other.admission()));
    assert!(harness.requests().is_empty(), "admission sends nothing");
}
