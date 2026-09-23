//! Deadlines, retries, single-attempt operations and cancellation evidence
//! (plan task S06), against a counting loopback server.
//!
//! Error bodies are labelled supplemental fixtures in the documented error
//! envelope shape (`kite-api-docs/docs/connect/v3/response-structure.md:17-28`).

mod support;

use std::time::{Duration, Instant};

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::{Credentials, KiteCredentials};
use manja::kite::connect::models::{Order, PositionConversionRequest};
use manja::kite::connect::scheduler::{PermitTarget, SchedulerLimits};
use manja::kite::error::{HttpErrorKind, TransportStage};

use support::fixtures;
use support::http::{HttpHarness, Reply};

fn client_with(base: &str, scheduler: SchedulerLimits) -> HTTPClient {
    let config = Config::from_parts(
        base,
        base,
        base,
        KiteCredentials::new("test_api_key", "test_api_secret", "", "", ""),
    )
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

// A response-shaped order for the legacy placement and modification entry
// points; S09 replaces them with dedicated request types.
fn legacy_order() -> Order {
    serde_json::from_value(serde_json::json!({
        "order_id": "151220000000000", "parent_order_id": null,
        "exchange_order_id": null, "modified": false, "placed_by": "AB1234",
        "variety": "regular", "status": "OPEN", "tradingsymbol": "INFY",
        "exchange": "NSE", "instrument_token": 408065, "transaction_type": "BUY",
        "order_type": "LIMIT", "product": "CNC", "validity": "DAY", "price": 1500.0,
        "quantity": 1, "trigger_price": 0.0, "average_price": 0.0,
        "pending_quantity": 1, "filled_quantity": 0, "disclosed_quantity": 0,
        "order_timestamp": null, "exchange_timestamp": null,
        "exchange_update_timestamp": null, "status_message": null,
        "status_message_raw": null, "cancelled_quantity": 0,
        "auction_number": null, "meta": {}, "tag": null, "guid": "g"
    }))
    .unwrap()
}

fn conversion() -> PositionConversionRequest {
    serde_json::from_value(serde_json::json!({
        "tradingsymbol": "INFY", "exchange": "NSE", "transaction_type": "BUY",
        "position_type": "overnight", "quantity": 1,
        "old_product": "CNC", "new_product": "MIS"
    }))
    .unwrap()
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
    once!("place", |c| c.orders().place_order(&legacy_order()));
    once!("modify", |c| c.orders().modify_order(
        "regular",
        "151220000000000",
        &legacy_order()
    ));
    once!("cancel", |c| c
        .orders()
        .cancel_order("regular", "151220000000000"));
    once!("convert", |c| {
        let c2 = c.clone();
        async move {
            manja::kite::connect::api::Portfolio::new(&c2)
                .convert_position(conversion())
                .await
        }
    });
    once!("exchange", |c| c
        .session()
        .generate_session("request_token"));
    once!("invalidate", |c| c.session().delete_session());
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
    let harness =
        HttpHarness::start(vec![Reply::Stall, Reply::Stall, Reply::Stall, Reply::Stall]).await;
    let tight = SchedulerLimits::default()
        .with_attempt_timeout(Duration::from_millis(300))
        .unwrap()
        .with_operation_deadline(Duration::from_secs(1))
        .unwrap()
        .with_read_attempts(5)
        .unwrap()
        .with_backoff(Duration::from_millis(10), Duration::from_millis(20))
        .unwrap();
    let start = Instant::now();
    let err = client_with(&harness.base_url(), tight)
        .user()
        .profile()
        .await
        .unwrap_err();
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
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
    let mut c = client(&harness.base_url());
    let query = [("i", "NSE:INFY")];
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
