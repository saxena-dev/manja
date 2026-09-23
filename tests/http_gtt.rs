//! GTT placement, modification, deletion and retrieval, against the
//! loopback harness.
//!
//! Baselines are the official `gtt_place_order.json`, `gtt_modify_order.json`,
//! `gtt_delete_order.json`, `gtt_get_orders.json` and `gtt_get_order.json`,
//! served unchanged. Request wire expectations are written out from the
//! documented examples (`kite:gtt.md:15-21,377-383,397-400`), and expected
//! values are written out by hand from the fixtures. The documented sandbox
//! does not offer GTT (`kite:sandbox.md:298`), so these are the only GTT
//! tests.

mod support;

use std::time::Duration;

use chrono::{FixedOffset, TimeZone};
use manja::kite::connect::admission::RateClass;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::{
    Exchange, GttOrderRequest, GttRequest, GttStatus, GttType, OrderType, OrderValidity,
    ProductType, TransactionType,
};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpErrorKind, ManjaError, TransportStage};
use manja::kite::obs::schema::{Endpoint, Method};
use manja::kite::protocol::{Inbound, InstrumentToken, Quantity};

use support::fixtures;
use support::http::{HttpHarness, RecordedRequest, Reply};

fn client(base: &str) -> HTTPClient {
    let scheduler = SchedulerLimits::default()
        .with_backoff(Duration::from_millis(10), Duration::from_millis(10))
        .unwrap()
        .with_jitter_seed(7);
    let config = Config::new(base).with_limits(HttpLimits::default().with_scheduler(scheduler));
    HTTPClient::with_config(config)
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

async fn serve(fixture: &str) -> HttpHarness {
    HttpHarness::start(vec![Reply::json(fixtures::json_body(fixture).unwrap())]).await
}

fn only(h: &HttpHarness) -> RecordedRequest {
    let [r] = h.requests().try_into().unwrap();
    r
}

/// Decode an `application/x-www-form-urlencoded` body.
fn form(body: &[u8]) -> Vec<(String, String)> {
    fn dec(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'+' => out.push(b' '),
                b'%' => {
                    out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                    i += 2;
                }
                c => out.push(c),
            }
            i += 1;
        }
        String::from_utf8(out).unwrap()
    }
    std::str::from_utf8(body)
        .unwrap()
        .split('&')
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap();
            (dec(k), dec(v))
        })
        .collect()
}

fn buy(price: f64) -> GttOrderRequest {
    GttOrderRequest::limit(
        TransactionType::BUY,
        Quantity::new(1).unwrap(),
        ProductType::CashAndCarry,
        price,
    )
}

fn single() -> GttRequest {
    // kite:gtt.md:19-21: INFY on NSE, trigger 702, last price 798, BUY 1 CNC LIMIT 702.5.
    GttRequest::single(Exchange::NSE, "INFY", 702.0, 798.0, buy(702.5))
}

fn is_validation(e: &ManjaError) -> bool {
    let h = e.as_http().unwrap();
    h.kind() == HttpErrorKind::Validation && h.stage() == TransportStage::NotStarted
}

fn ist(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> chrono::DateTime<FixedOffset> {
    FixedOffset::east_opt(5 * 3600 + 30 * 60)
        .unwrap()
        .with_ymd_and_hms(y, mo, d, h, mi, s)
        .unwrap()
}

#[tokio::test]
async fn placement_is_form_encoded_with_json_condition_and_orders() {
    let h = serve("gtt_place_order.json").await;
    let receipt = client(&h.base_url())
        .gtt()
        .place_trigger(&single())
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(receipt.trigger_id, 123);
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("POST", "/gtt/triggers")
    );
    assert_eq!(
        r.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    let fields = form(&r.body);
    let keys: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["type", "condition", "orders"]);
    assert_eq!(fields[0].1, "single");
    let condition: serde_json::Value = serde_json::from_str(&fields[1].1).unwrap();
    assert_eq!(
        condition,
        serde_json::json!({
            "exchange": "NSE", "tradingsymbol": "INFY",
            "trigger_values": [702.0], "last_price": 798.0
        })
    );
    let orders: serde_json::Value = serde_json::from_str(&fields[2].1).unwrap();
    assert_eq!(
        orders,
        serde_json::json!([{
            "exchange": "NSE", "tradingsymbol": "INFY", "transaction_type": "BUY",
            "quantity": 1, "order_type": "LIMIT", "product": "CNC", "price": 702.5
        }])
    );
}

#[tokio::test]
async fn modification_replaces_the_trigger_by_id() {
    let h = serve("gtt_modify_order.json").await;
    // kite:gtt.md:381-383: the same trigger with value 701.
    let request = GttRequest::single(Exchange::NSE, "INFY", 701.0, 798.0, buy(702.5));
    let receipt = client(&h.base_url())
        .gtt()
        .modify_trigger(123, &request)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(receipt.trigger_id, 123);
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("PUT", "/gtt/triggers/123")
    );
    let fields = form(&r.body);
    let condition: serde_json::Value = serde_json::from_str(&fields[1].1).unwrap();
    assert_eq!(condition["trigger_values"], serde_json::json!([701.0]));
}

#[tokio::test]
async fn deletion_sends_no_body() {
    let h = serve("gtt_delete_order.json").await;
    let receipt = client(&h.base_url())
        .gtt()
        .delete_trigger(123)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(receipt.trigger_id, 123);
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("DELETE", "/gtt/triggers/123")
    );
    assert!(r.body.is_empty());
    assert_eq!(r.header("content-type"), None);
}

#[tokio::test]
async fn the_trigger_list_decodes_both_documented_triggers() {
    let h = serve("gtt_get_orders.json").await;
    let triggers = client(&h.base_url())
        .gtt()
        .list_triggers()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(only(&h).target, "/gtt/triggers");
    assert_eq!(triggers.len(), 2);

    let t = &triggers[0];
    assert_eq!(t.id, 112127);
    assert_eq!(t.user_id, "XX0000");
    assert_eq!(t.parent_trigger, None);
    assert_eq!(t.trigger_type, Inbound::Known(GttType::Single));
    assert_eq!(t.status, Inbound::Known(GttStatus::Active));
    assert_eq!(t.created_at, Some(ist(2019, 9, 12, 13, 25, 16)));
    assert_eq!(t.updated_at, Some(ist(2019, 9, 12, 13, 25, 16)));
    assert_eq!(t.expires_at, Some(ist(2020, 9, 12, 13, 25, 16)));
    assert_eq!(t.condition.exchange, Inbound::Known(Exchange::NSE));
    assert_eq!(t.condition.tradingsymbol, "INFY");
    assert_eq!(t.condition.trigger_values, [702.0]);
    assert_eq!(t.condition.last_price, 798.0);
    assert_eq!(
        t.condition.instrument_token,
        Some(InstrumentToken::new(408065))
    );
    let [o] = t.orders.as_slice() else {
        panic!("one order")
    };
    assert_eq!(o.transaction_type, Inbound::Known(TransactionType::BUY));
    assert_eq!(o.order_type, Inbound::Known(OrderType::Limit));
    assert_eq!(o.product, Inbound::Known(ProductType::CashAndCarry));
    assert_eq!((o.quantity, o.price), (1, 702.5));
    assert_eq!(o.result, None);
    assert_eq!(t.meta, Some(serde_json::json!({})));

    let t = &triggers[1];
    assert_eq!(t.id, 105099);
    assert_eq!(t.trigger_type, Inbound::Known(GttType::TwoLeg));
    assert_eq!(t.status, Inbound::Known(GttStatus::Triggered));
    assert_eq!(t.created_at, Some(ist(2019, 9, 9, 15, 13, 22)));
    assert_eq!(t.updated_at, Some(ist(2019, 9, 9, 15, 15, 8)));
    assert_eq!(t.expires_at, Some(ist(2020, 1, 1, 12, 0, 0)));
    assert_eq!(t.condition.trigger_values, [102.0, 103.7]);
    assert_eq!(t.meta, None);
    assert_eq!(t.orders.len(), 2);
    assert_eq!(t.orders[0].result, None);
    let fired = t.orders[1].result.as_ref().unwrap();
    assert_eq!(fired.account_id, "XX0000");
    assert_eq!(fired.validity, Inbound::Known(OrderValidity::Day));
    assert_eq!(fired.triggered_at, 103.7);
    assert_eq!(fired.timestamp, Some(ist(2019, 9, 9, 15, 15, 8)));
    assert_eq!(
        fired.meta.as_deref(),
        Some(r#"{"app_id":12617,"gtt":105099}"#)
    );
    assert_eq!(fired.order_result.status, "failed");
    assert_eq!(fired.order_result.order_id, None, "an empty ID is no ID");
    assert!(fired
        .order_result
        .rejection_reason
        .as_deref()
        .unwrap()
        .starts_with("Your order price is lower than the current lower circuit limit"));
}

#[tokio::test]
async fn one_trigger_is_fetched_by_id() {
    let h = serve("gtt_get_order.json").await;
    let t = client(&h.base_url())
        .gtt()
        .get_trigger(123)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(only(&h).target, "/gtt/triggers/123");
    assert_eq!(t.id, 123);
    assert_eq!(t.trigger_type, Inbound::Known(GttType::TwoLeg));
    assert_eq!(t.condition.tradingsymbol, "RAIN");
    assert_eq!(
        t.condition.instrument_token,
        Some(InstrumentToken::new(3926273))
    );
    assert!(t.orders[1].result.is_some());
}

#[tokio::test]
async fn an_unknown_status_is_preserved() {
    // Supplemental, derived from gtt_get_order.json: status "paused".
    let body = fixtures::json_body("gtt_get_order.json")
        .unwrap()
        .replace("\"status\": \"triggered\"", "\"status\": \"paused\"");
    let h = HttpHarness::start(vec![Reply::json(body)]).await;
    let t = client(&h.base_url())
        .gtt()
        .get_trigger(123)
        .await
        .unwrap()
        .data
        .unwrap();
    assert!(t.status.is_unknown());
    assert_eq!(t.status.as_wire(), "paused");
}

#[tokio::test]
async fn invalid_requests_are_rejected_before_any_transport() {
    let h = HttpHarness::start(vec![]).await;
    let c = client(&h.base_url());
    let mut cases: Vec<(&str, GttRequest)> = Vec::new();
    let mut r = single();
    r.trigger_values.push(800.0);
    cases.push(("single with two values", r));
    let mut r = single();
    r.trigger_type = GttType::TwoLeg;
    r.trigger_values.push(800.0);
    cases.push(("two-leg with one order", r));
    let mut r = single();
    r.orders[0].order_type = OrderType::Market;
    cases.push(("MARKET order", r));
    let mut r = single();
    r.orders[0].price = f64::NAN;
    cases.push(("NaN price", r));
    let mut r = single();
    r.trigger_values[0] = 0.0;
    cases.push(("zero trigger", r));
    let mut r = single();
    r.last_price = -1.0;
    cases.push(("negative last price", r));
    let mut r = single();
    r.exchange = Exchange::INDICES;
    cases.push(("INDICES exchange", r));
    let mut r = single();
    r.tradingsymbol = String::new();
    cases.push(("empty symbol", r));
    for (name, request) in &cases {
        let err = c.gtt().place_trigger(request).await.unwrap_err();
        assert!(is_validation(&err), "{name}: {err:?}");
        let err = c.gtt().modify_trigger(1, request).await.unwrap_err();
        assert!(is_validation(&err), "{name} (modify): {err:?}");
    }
    assert!(
        h.requests().is_empty(),
        "no invalid request reached the wire"
    );
}

#[tokio::test]
async fn a_lost_placement_response_is_not_resubmitted() {
    let h = HttpHarness::start(vec![
        Reply::DropAfterRequest,
        Reply::json(fixtures::json_body("gtt_place_order.json").unwrap()),
    ])
    .await;
    let err = client(&h.base_url())
        .gtt()
        .place_trigger(&single())
        .await
        .unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.stage(), TransportStage::Started, "the broker may have it");
    assert_eq!(h.requests().len(), 1, "exactly one attempt");
}

#[tokio::test]
async fn deletion_after_a_429_is_not_retried() {
    let h = HttpHarness::start(vec![
        Reply::Respond {
            status: 429,
            content_type: "application/json",
            body: br#"{"status":"error","message":"Too many requests","error_type":"NetworkException"}"#
                .to_vec(),
        },
        Reply::json(fixtures::json_body("gtt_delete_order.json").unwrap()),
    ])
    .await;
    let err = client(&h.base_url())
        .gtt()
        .delete_trigger(123)
        .await
        .unwrap_err();
    assert!(err.as_http().is_some());
    assert_eq!(h.requests().len(), 1, "a mutation makes one attempt");
}

#[tokio::test]
async fn reads_retry_a_transient_failure() {
    // Supplemental: a 503 before the official list.
    let h = HttpHarness::start(vec![
        Reply::Respond {
            status: 503,
            content_type: "text/html",
            body: b"<html>unavailable</html>".to_vec(),
        },
        Reply::json(fixtures::json_body("gtt_get_orders.json").unwrap()),
    ])
    .await;
    let triggers = client(&h.base_url())
        .gtt()
        .list_triggers()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(triggers.len(), 2);
    assert_eq!(h.requests().len(), 2);
}

#[test]
fn gtt_endpoints_use_the_standard_quota_class() {
    for (m, e) in [
        (Method::Post, Endpoint::GttTriggers),
        (Method::Get, Endpoint::GttTriggers),
        (Method::Get, Endpoint::GttTriggersId),
        (Method::Put, Endpoint::GttTriggersId),
        (Method::Delete, Endpoint::GttTriggersId),
    ] {
        assert_eq!(RateClass::of(m, e), RateClass::Standard, "{m:?} {e:?}");
    }
    assert_eq!(Endpoint::GttTriggersId.as_str(), "/gtt/triggers/{id}");
}

fn sell(price: f64) -> GttOrderRequest {
    GttOrderRequest::limit(
        TransactionType::SELL,
        Quantity::new(1).unwrap(),
        ProductType::CashAndCarry,
        price,
    )
}

#[tokio::test]
async fn a_two_leg_placement_sends_both_values_and_both_orders() {
    let h = serve("gtt_place_order.json").await;
    // kite:gtt.md:129-167: INFY on NSE, triggers 702 and 798, last price 742,
    // SELL 1 CNC LIMIT at 702.5 and at 798.5.
    let request = GttRequest::two_leg(
        Exchange::NSE,
        "INFY",
        [702.0, 798.0],
        742.0,
        [sell(702.5), sell(798.5)],
    );
    client(&h.base_url())
        .gtt()
        .place_trigger(&request)
        .await
        .unwrap();
    let fields = form(&only(&h).body);
    assert_eq!(fields[0], ("type".to_string(), "two-leg".to_string()));
    let condition: serde_json::Value = serde_json::from_str(&fields[1].1).unwrap();
    assert_eq!(
        condition,
        serde_json::json!({
            "exchange": "NSE", "tradingsymbol": "INFY",
            "trigger_values": [702.0, 798.0], "last_price": 742.0
        })
    );
    let orders: serde_json::Value = serde_json::from_str(&fields[2].1).unwrap();
    assert_eq!(
        orders,
        serde_json::json!([
            {
                "exchange": "NSE", "tradingsymbol": "INFY", "transaction_type": "SELL",
                "quantity": 1, "order_type": "LIMIT", "product": "CNC", "price": 702.5
            },
            {
                "exchange": "NSE", "tradingsymbol": "INFY", "transaction_type": "SELL",
                "quantity": 1, "order_type": "LIMIT", "product": "CNC", "price": 798.5
            }
        ])
    );
}

#[tokio::test]
async fn a_fetched_trigger_converts_into_a_request_for_modification() {
    let get = serve("gtt_get_order.json").await;
    let c = client(&get.base_url());
    let trigger = c.gtt().get_trigger(123).await.unwrap().data.unwrap();
    let mut request = GttRequest::from_trigger(&trigger).unwrap();
    assert_eq!(request.trigger_type, GttType::TwoLeg);
    assert_eq!(request.exchange, Exchange::NSE);
    assert_eq!(request.tradingsymbol, "RAIN");
    assert_eq!(request.trigger_values, [102.0, 103.7]);
    assert_eq!(request.last_price, 102.6);
    assert_eq!(request.orders, [sell(1.0), sell(1.0)]);
    assert_eq!(request.validate(), Ok(()));

    // Change one value and send it back, as the documentation recommends.
    request.trigger_values[1] = 104.0;
    let put = serve("gtt_modify_order.json").await;
    client(&put.base_url())
        .gtt()
        .modify_trigger(trigger.id, &request)
        .await
        .unwrap();
    let r = only(&put);
    assert_eq!(r.target, "/gtt/triggers/123");
    let fields = form(&r.body);
    assert_eq!(fields[0].1, "two-leg");
    let condition: serde_json::Value = serde_json::from_str(&fields[1].1).unwrap();
    assert_eq!(
        condition["trigger_values"],
        serde_json::json!([102.0, 104.0])
    );
}

#[tokio::test]
async fn a_trigger_a_request_cannot_carry_does_not_convert() {
    let body = fixtures::json_body("gtt_get_order.json").unwrap();
    // Supplemental, derived from gtt_get_order.json: an unknown order type,
    // an unknown exchange in the condition, and one order for another symbol.
    let cases = [
        (
            "order_type",
            body.replacen("\"order_type\": \"LIMIT\"", "\"order_type\": \"STOP\"", 1),
        ),
        (
            "exchange",
            body.replacen(
                "\"exchange\": \"NSE\",\n            \"last_price\"",
                "\"exchange\": \"XNSE\",\n            \"last_price\"",
                1,
            ),
        ),
        (
            "orders",
            body.replacen(
                "\"tradingsymbol\": \"RAIN\",\n                \"product\"",
                "\"tradingsymbol\": \"INFY\",\n                \"product\"",
                1,
            ),
        ),
    ];
    for (field, body) in cases {
        assert_ne!(
            body,
            fixtures::json_body("gtt_get_order.json").unwrap(),
            "{field}: edit applied"
        );
        let h = HttpHarness::start(vec![Reply::json(body)]).await;
        let trigger = client(&h.base_url())
            .gtt()
            .get_trigger(123)
            .await
            .unwrap()
            .data
            .unwrap();
        let err = GttRequest::from_trigger(&trigger).unwrap_err();
        assert_eq!(err.field, field);
    }
}

#[test]
fn a_zero_last_price_is_accepted() {
    // The documentation sets no lower bound for the last price.
    let mut r = single();
    r.last_price = 0.0;
    assert_eq!(r.validate(), Ok(()));
}
