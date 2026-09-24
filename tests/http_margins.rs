//! Order margins, basket margins and the virtual contract note (order
//! charges), each a supported calculation (`docs/contract.md` §5).
//!
//! Baselines are the official `order_margins.json`, `basket_margins.json`
//! and `virtual_contract_note.json`, served unchanged. Request bodies are
//! asserted against the documented examples
//! (`kite:margins.md:21-36,138-167,350-388`).
//! Negative and partial responses are labelled supplements.

mod support;

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::{
    Exchange, OrderChargesRequest, OrderMarginRequest, OrderType, OrderVariety, ProductType,
    TransactionType,
};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpErrorKind, TransportStage};
use manja::kite::protocol::{Inbound, Quantity};

use support::fixtures;
use support::http::{HttpHarness, RecordedRequest, Reply};

fn client_with(base: &str, limits: HttpLimits) -> HTTPClient {
    let config = Config::new(base).with_limits(limits);
    HTTPClient::with_config(config)
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

fn fast() -> HttpLimits {
    HttpLimits::default().with_scheduler(
        SchedulerLimits::default()
            .with_backoff(
                std::time::Duration::from_millis(10),
                std::time::Duration::from_millis(20),
            )
            .unwrap(),
    )
}

fn only(h: &HttpHarness) -> RecordedRequest {
    let [r] = h.requests().try_into().unwrap();
    r
}

fn body_json(r: &RecordedRequest) -> serde_json::Value {
    serde_json::from_slice(&r.body).unwrap()
}

fn infy_market() -> OrderMarginRequest {
    OrderMarginRequest {
        exchange: Exchange::NSE,
        tradingsymbol: "INFY".into(),
        transaction_type: TransactionType::BUY,
        variety: OrderVariety::Regular,
        product: ProductType::CashAndCarry,
        order_type: OrderType::Market,
        quantity: Quantity::new(1).unwrap(),
        price: 0.0,
        trigger_price: 0.0,
    }
}

fn nifty(symbol: &str, side: TransactionType) -> OrderMarginRequest {
    OrderMarginRequest {
        exchange: Exchange::NFO,
        tradingsymbol: symbol.into(),
        transaction_type: side,
        variety: OrderVariety::Regular,
        product: ProductType::Normal,
        order_type: OrderType::Market,
        quantity: Quantity::new(75).unwrap(),
        price: 0.0,
        trigger_price: 0.0,
    }
}

fn charges_request() -> OrderChargesRequest {
    OrderChargesRequest {
        order_id: "111111111".into(),
        exchange: Exchange::NSE,
        tradingsymbol: "SBIN".into(),
        transaction_type: TransactionType::BUY,
        variety: OrderVariety::Regular,
        product: ProductType::CashAndCarry,
        order_type: OrderType::Market,
        quantity: Quantity::new(1).unwrap(),
        average_price: 560.0,
    }
}

#[tokio::test]
async fn order_margins_are_arrays_in_and_out() {
    let h = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("order_margins.json").unwrap(),
    )])
    .await;
    let c = client_with(&h.base_url(), fast());
    let margins = c
        .margins()
        .orders(&[infy_market()])
        .await
        .unwrap()
        .data
        .unwrap();
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("POST", "/margins/orders")
    );
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert_eq!(
        body_json(&r),
        serde_json::json!([{
            "exchange": "NSE", "tradingsymbol": "INFY", "transaction_type": "BUY",
            "variety": "regular", "product": "CNC", "order_type": "MARKET",
            "quantity": 1, "price": 0.0, "trigger_price": 0.0
        }])
    );
    assert_eq!(margins.len(), 1);
    let m = &margins[0];
    assert_eq!(m.r#type, "equity");
    assert_eq!(m.exchange, Inbound::Known(Exchange::NSE));
    assert_eq!(m.var, 1498.0);
    assert_eq!(m.leverage, 1.0);
    assert_eq!(m.total, 1498.0);
    assert_eq!(m.charges.transaction_tax_type, "stt");
    assert_eq!(m.charges.total, 1.79255122);
}

#[tokio::test]
async fn basket_margins_preserve_initial_final_and_per_order_entries() {
    let h = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("basket_margins.json").unwrap(),
    )])
    .await;
    let c = client_with(&h.base_url(), fast());
    let orders = [
        nifty("NIFTY23JUL20600CE", TransactionType::SELL),
        nifty("NIFTY23JUL20700CE", TransactionType::BUY),
    ];
    let basket = c
        .margins()
        .basket(&orders, true)
        .await
        .unwrap()
        .data
        .unwrap();
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("POST", "/margins/basket?consider_positions=true")
    );
    assert_eq!(body_json(&r).as_array().unwrap().len(), 2);
    assert_eq!(body_json(&r)[1]["tradingsymbol"], "NIFTY23JUL20700CE");
    assert_eq!(basket.initial.total, 96504.975);
    assert_eq!(basket.initial.exchange, Inbound::Known(Exchange::NONE));
    assert_eq!(basket.r#final.total, 34786.725000000006);
    assert_eq!(basket.r#final.option_premium, -2152.5);
    assert_eq!(basket.orders.len(), 2);
    assert_eq!(basket.orders[0].tradingsymbol, "NIFTY23JUL20600CE");
}

#[tokio::test]
async fn basket_without_positions_sets_the_query_flag() {
    let h = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("basket_margins.json").unwrap(),
    )])
    .await;
    let c = client_with(&h.base_url(), fast());
    c.margins().basket(&[infy_market()], false).await.unwrap();
    assert_eq!(only(&h).target, "/margins/basket?consider_positions=false");
}

#[tokio::test]
async fn virtual_contract_note_is_order_wise() {
    let h = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("virtual_contract_note.json").unwrap(),
    )])
    .await;
    let c = client_with(&h.base_url(), fast());
    let charges = c
        .charges()
        .orders(&[charges_request()])
        .await
        .unwrap()
        .data
        .unwrap();
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("POST", "/charges/orders")
    );
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert_eq!(
        body_json(&r),
        serde_json::json!([{
            "order_id": "111111111", "exchange": "NSE", "tradingsymbol": "SBIN",
            "transaction_type": "BUY", "variety": "regular", "product": "CNC",
            "order_type": "MARKET", "quantity": 1, "average_price": 560.0
        }])
    );
    // The fixture answers for its three documented example orders; the SDK
    // reports what it received and adds nothing.
    assert_eq!(charges.len(), 3);
    assert_eq!(charges[0].tradingsymbol, "SBIN");
    assert_eq!(charges[0].price, 560.0);
    assert_eq!(charges[0].charges.transaction_tax, 0.56);
    assert_eq!(charges[1].exchange, Inbound::Known(Exchange::MCX));
}

#[tokio::test]
async fn empty_and_invalid_requests_send_nothing() {
    let h = HttpHarness::start(vec![]).await;
    let c = client_with(&h.base_url(), fast());
    let empty: [OrderMarginRequest; 0] = [];
    let errs = [
        c.margins().orders(&empty).await.unwrap_err(),
        c.margins().basket(&empty, true).await.unwrap_err(),
        c.charges().orders(&[]).await.unwrap_err(),
        {
            let mut o = infy_market();
            o.exchange = Exchange::INDICES;
            c.margins().orders(&[o]).await.unwrap_err()
        },
        {
            let mut o = infy_market();
            o.price = -1.0;
            c.margins().orders(&[o]).await.unwrap_err()
        },
        {
            let mut o = charges_request();
            o.average_price = 0.0;
            c.charges().orders(&[o]).await.unwrap_err()
        },
    ];
    for e in errs {
        let e = e.as_http().unwrap();
        assert_eq!(
            (e.kind(), e.stage()),
            (HttpErrorKind::Validation, TransportStage::NotStarted)
        );
    }
    assert!(h.requests().is_empty());
}

#[tokio::test]
async fn an_oversized_request_body_is_rejected_before_admission() {
    let h = HttpHarness::start(vec![]).await;
    let limits = fast().with_request_body_bytes(1024).unwrap();
    let c = client_with(&h.base_url(), limits);
    let orders: Vec<_> = (0..20).map(|_| infy_market()).collect();
    let err = c.margins().orders(&orders).await.unwrap_err();
    assert_eq!(err.as_http().unwrap().kind(), HttpErrorKind::Validation);
    assert!(h.requests().is_empty());
}

#[tokio::test]
async fn malformed_and_partial_responses_are_errors_not_fabrications() {
    // Supplemental, derived from order_margins.json: `total` removed.
    let partial = fixtures::json_body("order_margins.json")
        .unwrap()
        .replace("\"total\": 1498", "\"totl\": 1498");
    // Supplemental, derived from basket_margins.json: `orders` removed.
    let no_orders = r#"{"status":"success","data":{"initial":null}}"#.to_string();
    for body in [partial, no_orders] {
        let h = HttpHarness::start(vec![Reply::json(body)]).await;
        let c = client_with(&h.base_url(), fast());
        let err = c
            .margins()
            .basket(&[infy_market()], true)
            .await
            .unwrap_err();
        assert_eq!(err.as_http().unwrap().kind(), HttpErrorKind::Decode);
    }
}

#[tokio::test]
async fn calculations_are_retried_as_read_like() {
    let h = HttpHarness::start(vec![
        Reply::Respond {
            status: 502,
            content_type: "text/html",
            body: b"<html>OMS down</html>".to_vec(),
        },
        Reply::json(fixtures::json_body("order_margins.json").unwrap()),
    ])
    .await;
    let c = client_with(&h.base_url(), fast());
    c.margins().orders(&[infy_market()]).await.unwrap();
    assert_eq!(h.requests().len(), 2, "one documented transient retry");
}
