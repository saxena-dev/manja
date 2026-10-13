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
    // Synchronous admission on the Control lane: refused at once with the
    // exact time until the first grant ages out (the D7 hand-over).
    assert_eq!(
        a.try_acquire(Priority::Control, RateClass::Standard, None)
            .unwrap_err(),
        AdmissionError::Refused {
            retry_after: Duration::from_secs(1)
        }
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
    assert_eq!(
        a.try_acquire(Priority::Control, RateClass::Standard, None)
            .unwrap_err()
            .retry_after(),
        Some(Duration::from_millis(2)),
        "the hint tracks the remaining time exactly"
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
    assert!(matches!(
        a.acquire_with_priority(Priority::Control, RateClass::OrderPlacement, None, WAIT)
            .await
            .unwrap_err(),
        AdmissionError::DailyCeilingReached { .. }
    ));
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
    assert_eq!(err.kind(), HttpErrorKind::Admission);
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

// ---- headroom and modifications remaining: advisory, the local view -----

use manja::kite::connect::admission::Headroom;
use manja::kite::connect::models::{ModifyOrderRequest, OrderType};

#[tokio::test(start_paused = true)]
async fn headroom_agrees_with_the_next_admission_decision() {
    let a = scope(AdmissionLimits::default());
    let start = tokio::time::Instant::now();
    let h = a.headroom(RateClass::Standard);
    assert_eq!(
        h,
        Headroom {
            standard_now: 8,
            control_now: 10,
            next_slot_in: None,
            day_remaining: None,
        }
    );
    // Reading reserves nothing: the same snapshot again.
    assert_eq!(a.headroom(RateClass::Standard), h);
    // Exactly standard_now Standard calls admit at once; the next waits.
    for n in (1..=h.standard_now).rev() {
        assert_eq!(a.headroom(RateClass::Standard).standard_now, n);
        admit(&a, Priority::Standard, RateClass::Standard).await;
    }
    assert_eq!(start.elapsed(), Duration::ZERO);
    let h = a.headroom(RateClass::Standard);
    assert_eq!(h.standard_now, 0);
    assert_eq!(h.control_now, 2, "the reserve is what Control has left");
    let wait = h.next_slot_in.expect("a window is full");
    assert_eq!(wait, Duration::from_secs(1));
    let ninth = a.acquire_with_priority(Priority::Standard, RateClass::Standard, None, WAIT);
    assert!(
        tokio::time::timeout(wait - Duration::from_millis(1), ninth)
            .await
            .is_err(),
        "the ninth Standard call waits"
    );
    // Control still admits its two, and then reports none.
    admit(&a, Priority::Control, RateClass::Standard).await;
    admit(&a, Priority::Control, RateClass::Standard).await;
    assert_eq!(a.headroom(RateClass::Standard).control_now, 0);
    // Advancing the clock by the hint makes a Standard slot appear.
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(start.elapsed(), wait);
    let h = a.headroom(RateClass::Standard);
    assert!(h.standard_now >= 1, "{h:?}");
    assert_eq!(h.next_slot_in, None);
    let before = tokio::time::Instant::now();
    admit(&a, Priority::Standard, RateClass::Standard).await;
    assert_eq!(before.elapsed(), Duration::ZERO);
    // A grant dropped unconsumed gives its slot back.
    let h = a.headroom(RateClass::Standard);
    let grant = a
        .acquire_with_priority(Priority::Standard, RateClass::Standard, None, WAIT)
        .await
        .unwrap();
    assert_eq!(
        a.headroom(RateClass::Standard).standard_now,
        h.standard_now - 1
    );
    drop(grant);
    assert_eq!(a.headroom(RateClass::Standard), h);
}

#[tokio::test(start_paused = true)]
async fn headroom_reports_equal_lanes_without_a_reserve_and_none_when_covered() {
    let a = scope(
        AdmissionLimits::default()
            .with_control_reserve(RateClass::Standard, 0)
            .unwrap()
            .with_control_reserve(RateClass::Quote, 1)
            .unwrap(),
    );
    let h = a.headroom(RateClass::Standard);
    assert_eq!((h.standard_now, h.control_now), (10, 10));
    // A reserve at the window rate: the Standard lane never admits, so no
    // slot is ever announced, while Control has the whole rate.
    let h = a.headroom(RateClass::Quote);
    assert_eq!(h.standard_now, 0);
    assert_eq!(h.control_now, 1);
    assert_eq!(h.next_slot_in, None);
    admit(&a, Priority::Control, RateClass::Quote).await;
    assert_eq!(a.headroom(RateClass::Quote).control_now, 0);
    assert_eq!(a.headroom(RateClass::Quote).next_slot_in, None);
}

#[tokio::test(start_paused = true)]
async fn day_remaining_counts_placements_down_to_the_ceiling() {
    let profile = QuotaProfile::kite_v3()
        .with_version("t")
        .with_daily_order_ceiling(3)
        .unwrap();
    let a = Admission::new(profile, AdmissionLimits::default());
    assert_eq!(a.headroom(RateClass::Standard).day_remaining, None);
    assert_eq!(a.headroom(RateClass::OrderModification).day_remaining, None);
    let h = a.headroom(RateClass::OrderPlacement);
    assert_eq!(h.day_remaining, Some(3));
    assert_eq!(
        (h.standard_now, h.control_now),
        (10, 10),
        "no reserve by default"
    );
    for left in [2, 1, 0] {
        admit(&a, Priority::Standard, RateClass::OrderPlacement).await;
        assert_eq!(
            a.headroom(RateClass::OrderPlacement).day_remaining,
            Some(left)
        );
    }
    // The ceiling refuses both lanes, and no window frees it.
    let h = a.headroom(RateClass::OrderPlacement);
    assert_eq!(
        h,
        Headroom {
            standard_now: 0,
            control_now: 0,
            next_slot_in: None,
            day_remaining: Some(0),
        }
    );
    assert!(matches!(
        a.acquire_with_priority(Priority::Control, RateClass::OrderPlacement, None, WAIT)
            .await
            .unwrap_err(),
        AdmissionError::DailyCeilingReached { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn modifications_remaining_counts_each_order_down_from_the_limit() {
    let a = scope(AdmissionLimits::default());
    let order_a = OrderId::new("151220000000000").unwrap();
    let order_b = OrderId::new("151220000000001").unwrap();
    assert_eq!(a.modifications_remaining(&order_a), 25);
    for _ in 0..2 {
        a.acquire_with_priority(
            Priority::Standard,
            RateClass::OrderModification,
            Some(order_a.as_str()),
            WAIT,
        )
        .await
        .unwrap()
        .consume();
    }
    assert_eq!(a.modifications_remaining(&order_a), 23);
    assert_eq!(a.modifications_remaining(&order_b), 25, "per order");
    // A grant dropped unconsumed is not a modification.
    let g = a
        .acquire_with_priority(
            Priority::Standard,
            RateClass::OrderModification,
            Some(order_a.as_str()),
            WAIT,
        )
        .await
        .unwrap();
    assert_eq!(a.modifications_remaining(&order_a), 22);
    drop(g);
    assert_eq!(a.modifications_remaining(&order_a), 23);
}

#[tokio::test]
async fn two_modifications_through_the_client_leave_twenty_three() {
    let modify = fixtures::json_body("order_modify.json").unwrap();
    let harness = HttpHarness::start(vec![Reply::json(modify.clone()), Reply::json(modify)]).await;
    let client = HTTPClient::builder(Config::new(&*harness.base_url()))
        .credentials(Credentials::new("k", "t").unwrap())
        .build()
        .unwrap();
    let order = OrderId::new("151220000000000").unwrap();
    let request = ModifyOrderRequest {
        order_type: Some(OrderType::Market),
        ..Default::default()
    };
    assert_eq!(client.admission().modifications_remaining(&order), 25);
    for _ in 0..2 {
        client
            .orders()
            .modify_order(OrderVariety::Regular, &order, &request)
            .await
            .unwrap();
    }
    assert_eq!(harness.requests().len(), 2);
    assert_eq!(client.admission().modifications_remaining(&order), 23);
    let h = client.admission().headroom(RateClass::OrderModification);
    assert_eq!(h.standard_now, 8, "two of the second's ten slots are used");
    assert_eq!(h.day_remaining, None);
}

// ---- mutations never wait: admitted now or refused ----------------------

use manja::kite::connect::models::{
    Exchange, OrderType as PlaceOrderType, PlaceOrderRequest, ProductType, Tag, TransactionType,
};
use manja::kite::obs::schema::AdmissionCause;
use manja::kite::protocol::Quantity;

fn market_buy() -> PlaceOrderRequest {
    PlaceOrderRequest::new(
        OrderVariety::Regular,
        Exchange::NSE,
        "INFY",
        TransactionType::BUY,
        PlaceOrderType::Market,
        Quantity::new(1).unwrap(),
        ProductType::CashAndCarry,
        Tag::new("t1").unwrap(),
    )
}

/// A client on `harness` whose admission scope reads the IST day from a
/// fixed wall clock, so a daily ceiling resets at a known instant.
struct FixedClock(i64);

impl manja::kite::connect::admission::WallClock for FixedClock {
    fn unix_seconds(&self) -> i64 {
        self.0
    }
}

// 2026-09-23 10:00:00 IST, as the admission unit tests use.
const MORNING: i64 = 1_790_137_800;

fn mutation_client(base: &str, profile: QuotaProfile) -> HTTPClient {
    let admission = Admission::with_clock(
        profile,
        AdmissionLimits::default(),
        std::sync::Arc::new(FixedClock(MORNING)),
    );
    HTTPClient::builder(Config::new(base))
        .admission(admission)
        .credentials(Credentials::new("k", "t").unwrap())
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_saturated_placement_window_refuses_at_once_with_an_accurate_hint() {
    let receipt = fixtures::json_body("order_response.json").unwrap();
    let harness = HttpHarness::start((0..11).map(|_| Reply::json(receipt.clone())).collect()).await;
    let client = mutation_client(&harness.base_url(), QuotaProfile::kite_v3());
    let start = tokio::time::Instant::now();
    for _ in 0..10 {
        client.orders().place_order(&market_buy()).await.unwrap();
    }
    assert_eq!(harness.requests().len(), 10);
    // The eleventh is refused now, with no wait and nothing on the wire.
    // The clock is paused around the call: a refusal awaits no timer, so no
    // paused time passes (the loopback server needs the real clock
    // otherwise).
    tokio::time::pause();
    let before = tokio::time::Instant::now();
    let err = client
        .orders()
        .place_order(&market_buy())
        .await
        .unwrap_err();
    assert_eq!(before.elapsed(), Duration::ZERO, "no timer was awaited");
    tokio::time::resume();
    let e = err;
    assert_eq!(e.kind(), HttpErrorKind::Admission);
    assert_eq!(e.stage(), TransportStage::NotStarted);
    assert!(!e.may_have_reached_broker());
    assert_eq!(e.admission_cause(), Some(AdmissionCause::WindowFull));
    assert_eq!(e.resets_at(), None);
    // Nothing was dispatched, so neither stamp exists; the identity and the
    // operation ID still do.
    assert_eq!((e.sent_at(), e.received_at()), (None, None));
    assert_eq!(e.source(), Some(client.source_identity()));
    assert!(e.operation_id().is_some());
    let hint = e.retry_after().expect("a full window carries retry_after");
    // The hint is the time until the first grant ages out of the window:
    // at most a second, and at least a second less the time already spent.
    assert!(
        hint > Duration::ZERO && hint <= Duration::from_secs(1),
        "{hint:?}"
    );
    assert!(
        hint + start.elapsed() >= Duration::from_secs(1),
        "{hint:?} after {:?}",
        start.elapsed()
    );
    assert_eq!(harness.requests().len(), 10, "the refusal sent nothing");
    // Waiting out the hint makes the next placement admit and send (the
    // exact instant is pinned at the Admission level below).
    tokio::time::sleep(hint).await;
    client.orders().place_order(&market_buy()).await.unwrap();
    assert_eq!(harness.requests().len(), 11);
}

#[tokio::test(start_paused = true)]
async fn try_acquire_refuses_with_the_instant_the_wait_path_would_sleep_to() {
    let a = scope(AdmissionLimits::default());
    let start = tokio::time::Instant::now();
    for _ in 0..10 {
        a.try_acquire(Priority::Standard, RateClass::OrderPlacement, None)
            .unwrap()
            .consume();
    }
    let err = a
        .try_acquire(Priority::Standard, RateClass::OrderPlacement, None)
        .unwrap_err();
    assert_eq!(
        err,
        AdmissionError::Refused {
            retry_after: Duration::from_secs(1)
        }
    );
    assert_eq!(err.cause(), AdmissionCause::WindowFull);
    assert_eq!(start.elapsed(), Duration::ZERO, "try_acquire never waits");
    assert_eq!(a.waiters(), 0, "and counts no waiter");
    // One millisecond short of the hint the lane is still full; at the hint
    // it admits, which is exactly where the wait path would have woken.
    tokio::time::advance(Duration::from_millis(999)).await;
    assert_eq!(
        a.try_acquire(Priority::Standard, RateClass::OrderPlacement, None)
            .unwrap_err()
            .retry_after(),
        Some(Duration::from_millis(1))
    );
    tokio::time::advance(Duration::from_millis(1)).await;
    a.try_acquire(Priority::Standard, RateClass::OrderPlacement, None)
        .unwrap()
        .consume();
}

#[tokio::test]
async fn a_read_in_the_same_scope_still_waits_for_its_slot() {
    let orders = fixtures::json_body("orders.json").unwrap();
    let harness = HttpHarness::start((0..9).map(|_| Reply::json(orders.clone())).collect()).await;
    let client = mutation_client(&harness.base_url(), QuotaProfile::kite_v3());
    let start = tokio::time::Instant::now();
    // Eight Standard-lane reads fill the Standard class; the ninth waits
    // under B-HTTP-06 and is sent once the first slot frees.
    for _ in 0..8 {
        client.orders().list_orders().await.unwrap();
    }
    let orders = client.orders();
    let ninth = tokio::time::timeout(Duration::from_millis(100), orders.list_orders()).await;
    assert!(ninth.is_err(), "the ninth read waits rather than refusing");
    assert_eq!(harness.requests().len(), 8);
    // Once the first slot frees the read is sent: it waited, it was not
    // refused.
    client.orders().list_orders().await.unwrap();
    assert_eq!(harness.requests().len(), 9);
    assert!(
        start.elapsed() >= Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn a_daily_ceiling_refusal_carries_when_it_resets() {
    let receipt = fixtures::json_body("order_response.json").unwrap();
    let harness =
        HttpHarness::start(vec![Reply::json(receipt.clone()), Reply::json(receipt)]).await;
    let profile = QuotaProfile::kite_v3()
        .with_version("t")
        .with_daily_order_ceiling(2)
        .unwrap();
    let client = mutation_client(&harness.base_url(), profile);
    for _ in 0..2 {
        client.orders().place_order(&market_buy()).await.unwrap();
    }
    tokio::time::pause();
    let before = tokio::time::Instant::now();
    let err = client
        .orders()
        .place_order(&market_buy())
        .await
        .unwrap_err();
    assert_eq!(before.elapsed(), Duration::ZERO, "no timer was awaited");
    tokio::time::resume();
    let e = err;
    assert_eq!(e.kind(), HttpErrorKind::Admission);
    assert_eq!(e.stage(), TransportStage::NotStarted);
    assert_eq!(e.admission_cause(), Some(AdmissionCause::DailyCeiling));
    assert_eq!(e.retry_after(), None, "no window frees a ceiling");
    // The next 00:00 IST after 2026-09-23 10:00 IST.
    assert_eq!(
        e.resets_at().unwrap().to_rfc3339(),
        "2026-09-24T00:00:00+05:30"
    );
    assert_eq!(harness.requests().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_twenty_sixth_modification_of_an_order_is_a_modification_ceiling() {
    let a = scope(AdmissionLimits::default());
    let order = OrderId::new("151220000000000").unwrap();
    for n in 0..25 {
        // Ten per second: step the clock every ten, so only the per-order
        // limit can refuse.
        if n > 0 && n % 10 == 0 {
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        a.try_acquire(
            Priority::Standard,
            RateClass::OrderModification,
            Some(order.as_str()),
        )
        .unwrap()
        .consume();
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    let err = a
        .try_acquire(
            Priority::Standard,
            RateClass::OrderModification,
            Some(order.as_str()),
        )
        .unwrap_err();
    assert_eq!(err, AdmissionError::ModificationCeiling);
    assert_eq!(err.cause(), AdmissionCause::ModificationCeiling);
    assert_eq!((err.retry_after(), err.resets_at()), (None, None));
    assert_eq!(a.modifications_remaining(&order), 0);
    // Through the client the same refusal arrives as an Admission error.
    let modify = fixtures::json_body("order_modify.json").unwrap();
    let harness = HttpHarness::start(vec![Reply::json(modify)]).await;
    let client = HTTPClient::builder(Config::new(&*harness.base_url()))
        .admission(a.clone())
        .credentials(Credentials::new("k", "t").unwrap())
        .build()
        .unwrap();
    let request = ModifyOrderRequest {
        order_type: Some(OrderType::Market),
        ..Default::default()
    };
    let err = client
        .orders()
        .modify_order(OrderVariety::Regular, &order, &request)
        .await
        .unwrap_err();
    let e = err;
    assert_eq!(e.kind(), HttpErrorKind::Admission);
    assert_eq!(
        e.admission_cause(),
        Some(AdmissionCause::ModificationCeiling)
    );
    assert!(harness.requests().is_empty());
}
