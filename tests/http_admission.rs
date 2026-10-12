//! Admission wired into the HTTP client: a request refused
//! by the shared budget scope fails before transport, with affirmative
//! `NotStarted` evidence and no request on the wire.

mod support;

use std::time::Duration;

use manja::kite::connect::admission::{Admission, AdmissionLimits, QuotaProfile};
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::LTPQuote;
use manja::kite::error::{HttpErrorKind, TransportStage};

use support::fixtures;
use support::http::{HttpHarness, Reply};

#[tokio::test]
async fn a_refused_admission_sends_nothing() {
    let harness = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("ltp.json").unwrap()),
        Reply::json(fixtures::json_body("ltp.json").unwrap()),
    ])
    .await;
    let base = harness.base_url();
    // A 1 ms admission wait: the second quote in the same second (quote
    // class, 1 per second) cannot be admitted in time.
    let admission = Admission::new(
        QuotaProfile::kite_v3(),
        AdmissionLimits::default()
            .with_wait(Duration::from_millis(1))
            .unwrap(),
    );
    let config = Config::new(&*base);
    let client = HTTPClient::builder(config)
        .admission(admission)
        .credentials(Credentials::new("k", "t").unwrap())
        .build()
        .unwrap();
    let query = ["NSE:INFY"];
    client
        .market()
        .get_quotes::<LTPQuote>(&query)
        .await
        .unwrap();
    let err = client
        .market()
        .get_quotes::<LTPQuote>(&query)
        .await
        .unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Admission);
    assert_eq!(e.stage(), TransportStage::NotStarted);
    assert!(!e.may_have_reached_broker());
    assert_eq!(
        harness.requests().len(),
        1,
        "the refused request was not sent"
    );
}

// ---- priority lanes and the control reserve (B-HTTP-15) -----------------
//
// Lane timing is pinned through the public `Admission` API under the paused
// clock, where a second is exact. The client-level checks below use a
// 60-second window so a real loopback server cannot make them depend on
// the clock.

use manja::kite::connect::admission::{AdmissionError, Priority, QuotaError, RateClass, Window};
use manja::kite::connect::models::OrderVariety;
use manja::kite::protocol::OrderId;

const WAIT: Duration = Duration::from_secs(60);

fn scope(limits: AdmissionLimits) -> Admission {
    Admission::new(QuotaProfile::kite_v3(), limits)
}

async fn admit(a: &Admission, priority: Priority, class: RateClass) {
    a.acquire_with_priority(priority, class, None, WAIT)
        .await
        .unwrap()
        .consume();
}

#[tokio::test(start_paused = true)]
async fn the_default_reserve_keeps_two_of_ten_standard_slots_for_control() {
    let a = scope(AdmissionLimits::default());
    assert_eq!(a.limits().control_reserve(RateClass::Standard), 2);
    let start = tokio::time::Instant::now();
    // Ten standard reads in one second: eight admit, the ninth waits a
    // full second for the first to age out.
    for _ in 0..8 {
        admit(&a, Priority::Standard, RateClass::Standard).await;
    }
    assert_eq!(start.elapsed(), Duration::ZERO, "eight admit at once");
    let ninth = a.acquire_with_priority(Priority::Standard, RateClass::Standard, None, WAIT);
    let ninth = tokio::time::timeout(Duration::from_millis(999), ninth).await;
    assert!(ninth.is_err(), "the ninth standard read waits");
    assert_eq!(a.waiters(), 0);
    // In that state control traffic admits immediately: a cancel, then a
    // read marked control.
    for _ in 0..2 {
        let before = tokio::time::Instant::now();
        admit(&a, Priority::Control, RateClass::Standard).await;
        assert_eq!(before.elapsed(), Duration::ZERO, "control admits at once");
    }
    // The window is now full for both lanes; the standard read is admitted
    // exactly when the oldest grant ages out.
    let before = tokio::time::Instant::now();
    admit(&a, Priority::Standard, RateClass::Standard).await;
    assert_eq!(
        start.elapsed(),
        Duration::from_secs(1),
        "the ninth standard read admits when the first grant ages out"
    );
    assert!(before.elapsed() <= Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn eleven_control_calls_in_one_second_refuse_the_eleventh() {
    // A 1 ms wait bound turns a wait into a refusal.
    let a = scope(
        AdmissionLimits::default()
            .with_wait(Duration::from_millis(1))
            .unwrap(),
    );
    let start = tokio::time::Instant::now();
    for _ in 0..10 {
        admit(&a, Priority::Control, RateClass::Standard).await;
    }
    assert_eq!(
        start.elapsed(),
        Duration::ZERO,
        "control may use the whole rate"
    );
    let err = a
        .acquire_with_priority(Priority::Control, RateClass::Standard, None, WAIT)
        .await
        .unwrap_err();
    assert_eq!(err, AdmissionError::WaitExpired, "the eleventh is refused");
    assert_eq!(
        start.elapsed(),
        Duration::ZERO,
        "refused at once, not after the wait bound"
    );
    // The refusal is accurate: the slot frees exactly one second after the
    // first grant, and not before.
    tokio::time::advance(Duration::from_millis(998)).await;
    assert!(
        a.acquire_with_priority(Priority::Control, RateClass::Standard, None, WAIT)
            .await
            .is_err(),
        "at 998 ms the slot is still 2 ms away, beyond the 1 ms wait bound"
    );
    tokio::time::advance(Duration::from_millis(2)).await;
    admit(&a, Priority::Control, RateClass::Standard).await;
    assert_eq!(start.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_reserve_of_zero_restores_the_single_lane() {
    let a = scope(
        AdmissionLimits::default()
            .with_control_reserve(RateClass::Standard, 0)
            .unwrap(),
    );
    let start = tokio::time::Instant::now();
    for _ in 0..10 {
        admit(&a, Priority::Standard, RateClass::Standard).await;
    }
    assert_eq!(
        start.elapsed(),
        Duration::ZERO,
        "ten standard admit at once"
    );
    // And the eleventh, on either lane, waits for the first to age out.
    admit(&a, Priority::Control, RateClass::Standard).await;
    assert_eq!(start.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn the_reserve_scales_with_the_window_period_and_skips_ceilings() {
    // A placement reserve of 2 per second reserves 8 of 10 per second and
    // 280 of 400 per minute; the daily ceiling is untouched.
    let profile = QuotaProfile::kite_v3()
        .with_version("t")
        .with_daily_order_ceiling(290)
        .unwrap();
    let a = Admission::new(
        profile,
        AdmissionLimits::default()
            .with_wait(WAIT)
            .unwrap()
            .with_control_reserve(RateClass::OrderPlacement, 2)
            .unwrap(),
    );
    let start = tokio::time::Instant::now();
    for _ in 0..280 {
        admit(&a, Priority::Standard, RateClass::OrderPlacement).await;
    }
    // 280 at 8 per second: 35 batches, the last starting at 34 s.
    assert_eq!(start.elapsed(), Duration::from_secs(34));
    // The 281st standard placement waits for the minute window, not the
    // second: it admits when the first grant is 60 s old.
    admit(&a, Priority::Standard, RateClass::OrderPlacement).await;
    assert_eq!(start.elapsed(), Duration::from_secs(60));
    // Control traffic is counted against the daily ceiling like any other:
    // 281 used, 9 remain.
    for _ in 0..9 {
        admit(&a, Priority::Control, RateClass::OrderPlacement).await;
    }
    assert_eq!(
        a.acquire_with_priority(Priority::Control, RateClass::OrderPlacement, None, WAIT)
            .await
            .unwrap_err(),
        AdmissionError::DailyCeilingReached
    );
}

#[test]
fn the_control_reserve_is_bounded_and_named() {
    let d = AdmissionLimits::default();
    for class in [
        RateClass::Quote,
        RateClass::Historical,
        RateClass::OrderPlacement,
        RateClass::OrderModification,
        RateClass::Unclassified,
    ] {
        assert_eq!(d.control_reserve(class), 0, "{class:?} defaults to 0");
    }
    assert_eq!(d.control_reserve(RateClass::Standard), 2);
    assert!(d.with_control_reserve(RateClass::Standard, 0).is_ok());
    assert_eq!(
        d.with_control_reserve(RateClass::Quote, 5)
            .unwrap()
            .control_reserve(RateClass::Quote),
        5
    );
    assert_eq!(
        d.with_control_reserve(RateClass::Standard, 6).unwrap_err(),
        QuotaError::Bound("B-HTTP-15")
    );
    assert!(
        d.with_control_reserve(RateClass::Historical, u32::MAX)
            .unwrap_err()
            .to_string()
            .contains("B-HTTP-15")
    );
}

#[tokio::test(start_paused = true)]
async fn a_reserve_at_the_window_rate_refuses_standard_traffic_at_once() {
    // Quotes admit 1 per second; a reserve of 1 leaves the standard lane
    // nothing, and the refusal is immediate rather than a wait.
    let a = scope(
        AdmissionLimits::default()
            .with_control_reserve(RateClass::Quote, 1)
            .unwrap(),
    );
    let start = tokio::time::Instant::now();
    let err = a
        .acquire_with_priority(Priority::Standard, RateClass::Quote, None, WAIT)
        .await
        .unwrap_err();
    assert_eq!(err, AdmissionError::NoStandardCapacity);
    assert!(err.to_string().contains("B-HTTP-15"));
    assert_eq!(start.elapsed(), Duration::ZERO);
    assert_eq!(a.waiters(), 0);
    admit(&a, Priority::Control, RateClass::Quote).await;
}

// A client whose standard class admits `limit` per minute with a reserve of
// `reserve` per second, which over the minute reserves 60 × `reserve` slots:
// 61 per minute with a reserve of 1 leaves the standard lane one slot and
// the control lane all 61.
fn lane_client(base: &str, limit: u32, reserve: u32) -> (HTTPClient, Admission) {
    let profile = QuotaProfile::kite_v3()
        .with_version("lanes")
        .with_windows(
            RateClass::Standard,
            vec![Window::new(limit, Duration::from_secs(60))],
        )
        .unwrap();
    let admission = Admission::new(
        profile,
        AdmissionLimits::default()
            .with_wait(Duration::from_millis(1))
            .unwrap()
            .with_control_reserve(RateClass::Standard, reserve)
            .unwrap(),
    );
    let client = HTTPClient::builder(Config::new(base))
        .admission(admission.clone())
        .credentials(Credentials::new("k", "t").unwrap())
        .build()
        .unwrap();
    (client, admission)
}

#[tokio::test]
async fn cancels_and_marked_reads_take_the_control_lane_through_the_client() {
    let orders = fixtures::json_body("orders.json").unwrap();
    let harness = HttpHarness::start(vec![
        Reply::json(orders.clone()),
        Reply::json(fixtures::json_body("order_cancel.json").unwrap()),
        Reply::json(orders),
    ])
    .await;
    let (client, _) = lane_client(&harness.base_url(), 61, 1);
    // Standard lane: one slot. The first read is sent, the second is
    // refused before transport.
    client.orders().list_orders().await.unwrap();
    let err = client.orders().list_orders().await.unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Admission);
    assert_eq!(e.stage(), TransportStage::NotStarted);
    assert_eq!(harness.requests().len(), 1);
    // A cancel is control by default: it is admitted and sent.
    client
        .orders()
        .cancel_order(
            OrderVariety::Regular,
            &OrderId::new("151220000000000").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(harness.requests().len(), 2);
    // A read marked control is admitted and sent.
    client
        .with_priority(Priority::Control)
        .orders()
        .list_orders()
        .await
        .unwrap();
    assert_eq!(harness.requests().len(), 3);
    // The control lane self-limits at the class rate; the paused-clock test
    // above pins that, with the exact second the eleventh call waits for.
}

#[tokio::test]
async fn a_mark_overrides_the_default_in_both_directions() {
    let harness = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("order_trades.json").unwrap()),
        Reply::json(fixtures::json_body("order_cancel.json").unwrap()),
    ])
    .await;
    let (client, _) = lane_client(&harness.base_url(), 61, 1);
    let id = OrderId::new("151220000000000").unwrap();
    // The one standard slot goes to a read.
    client.orders().get_order_trades(&id).await.unwrap();
    // A cancel marked standard is refused like any other standard call ...
    let err = client
        .with_priority(Priority::Standard)
        .orders()
        .cancel_order(OrderVariety::Regular, &id)
        .await
        .unwrap_err();
    assert_eq!(err.as_http().unwrap().kind(), HttpErrorKind::Admission);
    assert_eq!(harness.requests().len(), 1);
    // ... while the unmarked cancel takes the control lane.
    client
        .orders()
        .cancel_order(OrderVariety::Regular, &id)
        .await
        .unwrap();
    assert_eq!(harness.requests().len(), 2);
}

#[tokio::test]
async fn a_reserve_covering_the_class_refuses_standard_calls_naming_the_bound() {
    let harness = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("orders.json").unwrap(),
    )])
    .await;
    // Three per minute with a reserve of 1 per second: 60 slots reserved of 3.
    let (client, _) = lane_client(&harness.base_url(), 3, 1);
    let err = client.orders().list_orders().await.unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Admission);
    assert_eq!(e.stage(), TransportStage::NotStarted);
    assert!(e.detail().is_some_and(|d| d.as_str().contains("B-HTTP-15")));
    assert!(harness.requests().is_empty(), "nothing was sent");
    client
        .with_priority(Priority::Control)
        .orders()
        .list_orders()
        .await
        .unwrap();
    assert_eq!(harness.requests().len(), 1);
}
