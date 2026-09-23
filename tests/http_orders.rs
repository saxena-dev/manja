//! Validated wire requests for placement, modification, cancellation and
//! position conversion.
//!
//! Acknowledgement baselines are the official `order_response.json`,
//! `order_modify.json`, `order_cancel.json` and `convert_position.json`,
//! served unchanged. Request wire expectations are written out from the
//! documented examples (`kite:orders.md:58-162`,
//! `kite:portfolio.md:463-497`), independently of the fixtures.

mod support;

use std::time::Duration;

use manja::kite::connect::api::Portfolio;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::{
    Exchange, ModifyOrderRequest, OrderType, OrderValidity, OrderVariety, PlaceOrderRequest,
    PositionConversionRequest, PositionType, ProductType, SliceResult, TransactionType,
};
use manja::kite::connect::scheduler::{PermitTarget, SchedulerLimits};
use manja::kite::error::{HttpErrorKind, ManjaError, TransportStage};
use manja::kite::protocol::Quantity;

use support::fixtures;
use support::http::{HttpHarness, RecordedRequest, Reply};

fn client_with(base: &str, scheduler: SchedulerLimits) -> HTTPClient {
    let config = Config::new(base).with_limits(HttpLimits::default().with_scheduler(scheduler));
    HTTPClient::with_config(config)
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

fn client(base: &str) -> HTTPClient {
    client_with(base, SchedulerLimits::default())
}

async fn serve(fixture: &str) -> HttpHarness {
    HttpHarness::start(vec![Reply::json(fixtures::json_body(fixture).unwrap())]).await
}

fn only(h: &HttpHarness) -> RecordedRequest {
    let [r] = h.requests().try_into().unwrap();
    r
}

fn market_buy() -> PlaceOrderRequest {
    // kite:orders.md:58-70: ACC, NSE, BUY, MARKET, 1, MIS, DAY, market_protection -1.
    let mut r = PlaceOrderRequest::new(
        OrderVariety::Regular,
        Exchange::NSE,
        "ACC",
        TransactionType::BUY,
        OrderType::Market,
        Quantity::new(1).unwrap(),
        ProductType::MarginIntradaySquareoff,
    );
    r.market_protection = Some(-1.0);
    r
}

fn limit_buy() -> PlaceOrderRequest {
    let mut r = PlaceOrderRequest::new(
        OrderVariety::Regular,
        Exchange::NSE,
        "INFY",
        TransactionType::BUY,
        OrderType::Limit,
        Quantity::new(1).unwrap(),
        ProductType::CashAndCarry,
    );
    r.price = Some(1500.5);
    r
}

fn is_validation(e: &ManjaError) -> bool {
    let h = e.as_http().unwrap();
    h.kind() == HttpErrorKind::Validation && h.stage() == TransportStage::NotStarted
}

#[tokio::test]
async fn placement_is_form_encoded_and_acknowledged() {
    let h = serve("order_response.json").await;
    let ack = client(&h.base_url())
        .orders()
        .place_order(&market_buy())
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(ack.order_id, "151220000000000");
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("POST", "/orders/regular")
    );
    assert_eq!(
        r.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        String::from_utf8(r.body).unwrap(),
        "tradingsymbol=ACC&exchange=NSE&transaction_type=BUY&order_type=MARKET&quantity=1\
         &product=MIS&validity=DAY&market_protection=-1"
    );
}

#[tokio::test]
async fn modification_sends_only_the_set_fields() {
    let h = serve("order_modify.json").await;
    // kite:orders.md:109-117: order_type MARKET, quantity 3, validity DAY.
    let request = ModifyOrderRequest {
        order_type: Some(OrderType::Market),
        quantity: Some(Quantity::new(3).unwrap()),
        validity: Some(OrderValidity::Day),
        market_protection: Some(-1.0),
        ..Default::default()
    };
    let ack = client(&h.base_url())
        .orders()
        .modify_order(OrderVariety::Regular, "151220000000000", &request)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(ack.order_id, "151220000000000");
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("PUT", "/orders/regular/151220000000000")
    );
    assert_eq!(
        String::from_utf8(r.body).unwrap(),
        "order_type=MARKET&quantity=3&validity=DAY&market_protection=-1"
    );
}

#[tokio::test]
async fn cancellation_puts_no_credential_in_the_url() {
    let h = serve("order_cancel.json").await;
    let ack = client(&h.base_url())
        .orders()
        .cancel_order(OrderVariety::Regular, "151220000000000")
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(ack.order_id, "151220000000000");
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("DELETE", "/orders/regular/151220000000000")
    );
    assert!(!r.target.contains('?'), "BR-08: no query at all");
    assert!(r.body.is_empty());
    assert_eq!(r.header("content-type"), None);
    assert_eq!(
        r.header("authorization"),
        Some("token test_api_key:test_access_token")
    );
}

#[tokio::test]
async fn position_conversion_is_form_encoded() {
    let h = serve("convert_position.json").await;
    let c = client(&h.base_url());
    // kite:portfolio.md:468-477.
    let request = PositionConversionRequest {
        tradingsymbol: "INFY".into(),
        exchange: Exchange::NSE,
        transaction_type: TransactionType::BUY,
        position_type: PositionType::Overnight,
        quantity: Quantity::new(3).unwrap(),
        old_product: ProductType::Normal,
        new_product: ProductType::MarginIntradaySquareoff,
    };
    let converted = Portfolio::new(&c)
        .convert_position(&request)
        .await
        .unwrap()
        .data
        .unwrap();
    assert!(converted);
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("PUT", "/portfolio/positions")
    );
    assert_eq!(
        String::from_utf8(r.body).unwrap(),
        "tradingsymbol=INFY&exchange=NSE&transaction_type=BUY&position_type=overnight\
         &quantity=3&old_product=NRML&new_product=MIS"
    );
}

#[tokio::test]
async fn invalid_requests_are_rejected_before_any_transport() {
    let h = HttpHarness::start(vec![]).await;
    let c = client(&h.base_url());
    let mut cases: Vec<(&str, PlaceOrderRequest)> = Vec::new();
    let mut r = limit_buy();
    r.price = None;
    cases.push(("LIMIT without price", r));
    let mut r = market_buy();
    r.price = Some(10.0);
    cases.push(("MARKET with price", r));
    let mut r = limit_buy();
    r.order_type = OrderType::Stoploss;
    cases.push(("SL without trigger", r));
    let mut r = limit_buy();
    r.order_type = OrderType::StoplossMarket;
    r.trigger_price = Some(1.0);
    cases.push(("SL-M with price", r));
    let mut r = limit_buy();
    r.trigger_price = Some(1.0);
    cases.push(("LIMIT with trigger", r));
    let mut r = limit_buy();
    r.price = Some(f64::NAN);
    cases.push(("NaN price", r));
    let mut r = limit_buy();
    r.price = Some(-1.0);
    cases.push(("negative price", r));
    let mut r = limit_buy();
    r.validity = OrderValidity::TimeToLive;
    cases.push(("TTL without minutes", r));
    let mut r = limit_buy();
    r.validity_ttl = Some(5);
    cases.push(("minutes without TTL", r));
    let mut r = limit_buy();
    r.variety = OrderVariety::Iceberg;
    cases.push(("iceberg without legs", r));
    let mut r = limit_buy();
    r.variety = OrderVariety::Iceberg;
    r.iceberg_legs = Some(51);
    r.iceberg_quantity = Some(1);
    cases.push(("51 iceberg legs", r));
    let mut r = limit_buy();
    r.iceberg_legs = Some(2);
    cases.push(("legs outside iceberg", r));
    let mut r = limit_buy();
    r.variety = OrderVariety::Auction;
    cases.push(("auction without number", r));
    let mut r = limit_buy();
    r.auction_number = Some("20".into());
    cases.push(("number outside auction", r));
    let mut r = limit_buy();
    r.tag = Some("has space".into());
    cases.push(("tag with space", r));
    let mut r = limit_buy();
    r.tag = Some("x".repeat(21));
    cases.push(("21-char tag", r));
    let mut r = limit_buy();
    r.exchange = Exchange::INDICES;
    cases.push(("INDICES exchange", r));
    let mut r = limit_buy();
    r.tradingsymbol = String::new();
    cases.push(("empty symbol", r));
    let mut r = limit_buy();
    r.disclosed_quantity = Some(2);
    cases.push(("disclosed above quantity", r));
    let mut r = market_buy();
    r.market_protection = Some(150.0);
    cases.push(("protection above 100", r));
    let mut r = limit_buy();
    r.market_protection = Some(5.0);
    cases.push(("protection on LIMIT", r));
    for (name, request) in &cases {
        let err = c.orders().place_order(request).await.unwrap_err();
        assert!(is_validation(&err), "{name}: {err:?}");
    }
    let orders = c.orders();
    let empty = ModifyOrderRequest::default();
    assert!(is_validation(
        &orders
            .modify_order(OrderVariety::Regular, "1", &empty)
            .await
            .unwrap_err()
    ));
    let co_quantity = ModifyOrderRequest {
        quantity: Some(Quantity::new(2).unwrap()),
        ..Default::default()
    };
    assert!(is_validation(
        &orders
            .modify_order(OrderVariety::Cover, "1", &co_quantity)
            .await
            .unwrap_err()
    ));
    for bad_id in ["", "1/../2", "1?x=y", "12 3"] {
        assert!(is_validation(
            &orders
                .cancel_order(OrderVariety::Regular, bad_id)
                .await
                .unwrap_err()
        ));
    }
    let same_product = PositionConversionRequest {
        tradingsymbol: "INFY".into(),
        exchange: Exchange::NSE,
        transaction_type: TransactionType::BUY,
        position_type: PositionType::Day,
        quantity: Quantity::new(1).unwrap(),
        old_product: ProductType::Normal,
        new_product: ProductType::Normal,
    };
    assert!(is_validation(
        &Portfolio::new(&c)
            .convert_position(&same_product)
            .await
            .unwrap_err()
    ));
    assert!(
        h.requests().is_empty(),
        "no invalid request reached the wire"
    );
    assert_eq!(c.admission().waiters(), 0);
}

#[tokio::test]
async fn a_lost_placement_response_is_not_resubmitted() {
    let h = HttpHarness::start(vec![
        Reply::DropAfterRequest,
        Reply::json(fixtures::json_body("order_response.json").unwrap()),
    ])
    .await;
    let err = client(&h.base_url())
        .orders()
        .place_order(&limit_buy())
        .await
        .unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.stage(), TransportStage::Started, "the broker may have it");
    assert!(e.may_have_reached_broker());
    assert_eq!(h.requests().len(), 1, "exactly one attempt");
}

#[tokio::test]
async fn a_malformed_acknowledgement_is_evidence_not_success() {
    // Supplemental, derived from order_response.json: the order_id key is
    // misspelt, so the acknowledgement lacks its only field.
    let h = HttpHarness::start(vec![Reply::json(
        br#"{"status":"success","data":{"orderid":"151220000000000"}}"#.to_vec(),
    )])
    .await;
    let err = client(&h.base_url())
        .orders()
        .place_order(&limit_buy())
        .await
        .unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Decode);
    assert_eq!(e.http_status(), Some(200));
    assert_eq!(h.requests().len(), 1);
}

#[tokio::test]
async fn a_permit_separates_admission_from_dispatch() {
    let h = serve("order_response.json").await;
    let c = client(&h.base_url());
    let permit = c.admit(PermitTarget::PlaceOrder).await.unwrap();
    // The caller's own checks run here; nothing has been sent.
    assert!(h.requests().is_empty());
    c.orders()
        .place_order_with_permit(&limit_buy(), permit)
        .await
        .unwrap();
    assert_eq!(h.requests().len(), 1);
}

#[tokio::test]
async fn a_mismatched_or_expired_permit_cannot_start_transport() {
    let h = HttpHarness::start(vec![]).await;
    let c = client_with(
        &h.base_url(),
        SchedulerLimits::default()
            .with_permit_validity(Duration::from_millis(20))
            .unwrap(),
    );
    let wrong = c.admit(PermitTarget::CancelOrder).await.unwrap();
    let e = c
        .orders()
        .place_order_with_permit(&limit_buy(), wrong)
        .await
        .unwrap_err();
    assert_eq!(e.as_http().unwrap().kind(), HttpErrorKind::Admission);
    let permit = c.admit(PermitTarget::PlaceOrder).await.unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    let e = c
        .orders()
        .place_order_with_permit(&limit_buy(), permit)
        .await
        .unwrap_err();
    assert_eq!(e.as_http().unwrap().kind(), HttpErrorKind::Deadline);
    let other = client(&h.base_url());
    let foreign = other.admit(PermitTarget::PlaceOrder).await.unwrap();
    let e = c
        .orders()
        .place_order_with_permit(&limit_buy(), foreign)
        .await
        .unwrap_err();
    assert_eq!(e.as_http().unwrap().kind(), HttpErrorKind::Admission);
    assert!(h.requests().is_empty());
}

#[tokio::test]
async fn a_sliced_placement_reports_every_slice_including_failures() {
    // Official autoslice_response.json: a parent order and four further
    // slices, one rejected for margin.
    let h = serve("autoslice_response.json").await;
    let mut request = market_buy();
    request.autoslice = Some(true);
    let receipt = client(&h.base_url())
        .orders()
        .place_order(&request)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(receipt.order_id, "1914227164488687616");
    assert_eq!(receipt.slices.len(), 4);
    assert!(
        receipt.has_failed_slices(),
        "a failed slice is never hidden"
    );
    match &receipt.slices[2] {
        SliceResult::Failed(e) => {
            assert_eq!(e.code, Some(400));
            assert_eq!(e.error_type.as_ref().unwrap().as_wire(), "MarginException");
            assert!(e
                .message
                .as_deref()
                .unwrap()
                .starts_with("Insufficient funds"));
        }
        other => panic!("{other:?}"),
    }
    assert!(String::from_utf8(only(&h).body)
        .unwrap()
        .contains("autoslice=true"));

    // Supplemental, derived from the same fixture: the array form the
    // documentation shows (kite:orders.md:548-560), whose first entry is the
    // same order. It decodes to the same receipt.
    let array = r#"{"status":"success","data":[
        {"order_id":"1914227164488687616"},
        {"order_id":"1914227164534824960"},
        {"order_id":"1914227164580962304"},
        {"error":{"code":400,"error_type":"MarginException","message":"Insufficient funds. Required margin is 228365.92 but available margin is 228358.50.","data":null}},
        {"order_id":"1914227164681625600"}
    ]}"#;
    let h = HttpHarness::start(vec![Reply::json(array)]).await;
    let same = client(&h.base_url())
        .orders()
        .place_order(&request)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(same, receipt);

    // A plain placement has no slices.
    let h = serve("order_response.json").await;
    let plain = client(&h.base_url())
        .orders()
        .place_order(&market_buy())
        .await
        .unwrap()
        .data
        .unwrap();
    assert!(plain.slices.is_empty() && !plain.has_failed_slices());

    // A slice with neither an order ID nor an error is evidence, not
    // success.
    let bad = r#"{"status":"success","data":{"order_id":"1","children":[{"code":1}]}}"#;
    let h = HttpHarness::start(vec![Reply::json(bad)]).await;
    let err = client(&h.base_url())
        .orders()
        .place_order(&request)
        .await
        .unwrap_err();
    assert_eq!(err.as_http().unwrap().kind(), HttpErrorKind::Decode);
}
