//! Holdings authorisation at the depository (`kite:portfolio.md:503-541`),
//! against the loopback harness.
//!
//! The baseline is the official `holdings_auth.json`, served unchanged. The
//! request wire expectation is written out from the documented example
//! (`kite:portfolio.md:519-524`).

mod support;

use manja::kite::connect::admission::RateClass;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::{ApiKey, Credentials};
use manja::kite::connect::models::{
    Exchange, HoldingsAuthorisation, HoldingsAuthorisationRequest, OrderType, OrderVariety,
    PlaceOrderRequest, ProductType, TransactionType,
};
use manja::kite::connect::scheduler::RetryClass;
use manja::kite::error::{HttpErrorKind, TransportStage};
use manja::kite::obs::schema::{Endpoint, Method};
use manja::kite::protocol::Quantity;

use support::fixtures;
use support::http::{HttpHarness, RecordedRequest, Reply};

fn client(base: &str) -> HTTPClient {
    HTTPClient::with_config(Config::new(base))
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

fn only(h: &HttpHarness) -> RecordedRequest {
    let [r] = h.requests().try_into().unwrap();
    r
}

fn two_isins() -> HoldingsAuthorisationRequest {
    // kite:portfolio.md:522-523: INE002A01018 and INE009A01021, 50 each.
    HoldingsAuthorisationRequest::all()
        .with("INE002A01018", Quantity::new(50).unwrap())
        .with("INE009A01021", Quantity::new(50).unwrap())
}

#[tokio::test]
async fn named_holdings_are_sent_as_isin_and_quantity_pairs() {
    let h = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("holdings_auth.json").unwrap(),
    )])
    .await;
    let auth = client(&h.base_url())
        .portfolio()
        .authorise_holdings(&two_isins())
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(auth.request_id, "na8QgCeQm05UHG6NL9sAGRzdfSF64UdB");
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("POST", "/portfolio/holdings/authorise")
    );
    assert_eq!(
        r.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        String::from_utf8(r.body).unwrap(),
        "isin=INE002A01018&quantity=50&isin=INE009A01021&quantity=50"
    );
    // kite:portfolio.md:537.
    assert_eq!(
        auth.portal_url(&ApiKey::new("test_api_key").unwrap()),
        "https://kite.zerodha.com/connect/portfolio/authorise/holdings/test_api_key/na8QgCeQm05UHG6NL9sAGRzdfSF64UdB"
    );
}

#[tokio::test]
async fn the_entire_holdings_are_requested_with_no_body() {
    let h = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("holdings_auth.json").unwrap(),
    )])
    .await;
    client(&h.base_url())
        .portfolio()
        .authorise_holdings(&HoldingsAuthorisationRequest::all())
        .await
        .unwrap();
    let r = only(&h);
    assert_eq!(r.target, "/portfolio/holdings/authorise");
    assert!(r.body.is_empty());
    assert_eq!(r.header("content-type"), None);
}

#[tokio::test]
async fn a_malformed_isin_sends_nothing() {
    let h = HttpHarness::start(vec![]).await;
    let c = client(&h.base_url());
    for bad in [
        "ine002a01018",
        "INE002A0101",
        "INE002A010189",
        "INE002A0101/",
        "",
    ] {
        let request = HoldingsAuthorisationRequest::all().with(bad, Quantity::new(1).unwrap());
        let err = c
            .portfolio()
            .authorise_holdings(&request)
            .await
            .unwrap_err();
        let e = err.as_http().unwrap();
        assert_eq!(e.kind(), HttpErrorKind::Validation, "{bad:?}");
        assert_eq!(e.stage(), TransportStage::NotStarted);
        assert_eq!(e.endpoint(), Endpoint::HoldingsAuthorise);
    }
    assert!(h.requests().is_empty());
}

#[tokio::test]
async fn a_lost_response_is_not_resubmitted() {
    let h = HttpHarness::start(vec![
        Reply::DropAfterRequest,
        Reply::json(fixtures::json_body("holdings_auth.json").unwrap()),
    ])
    .await;
    let err = client(&h.base_url())
        .portfolio()
        .authorise_holdings(&two_isins())
        .await
        .unwrap_err();
    assert_eq!(err.as_http().unwrap().stage(), TransportStage::Started);
    assert_eq!(h.requests().len(), 1, "exactly one attempt");
}

#[tokio::test]
async fn a_sell_order_refused_with_428_asks_for_authorisation() {
    // Supplemental: the documented refusal, "10 quantity needs authorisation
    // at depository." with HTTP 428 (kite:portfolio.md:512), in the
    // documented error envelope (kite:response-structure.md:17-28). It has
    // no error_type, because the documentation gives none for this refusal.
    let h = HttpHarness::start(vec![Reply::Respond {
        status: 428,
        content_type: "application/json",
        body: br#"{"status":"error","message":"10 quantity needs authorisation at depository."}"#
            .to_vec(),
    }])
    .await;
    let mut sell = PlaceOrderRequest::new(
        OrderVariety::Regular,
        Exchange::NSE,
        "INFY",
        TransactionType::SELL,
        OrderType::Limit,
        Quantity::new(10).unwrap(),
        ProductType::CashAndCarry,
    );
    sell.price = Some(1500.0);
    let err = client(&h.base_url())
        .orders()
        .place_order(&sell)
        .await
        .unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.http_status(), Some(428));
    assert!(e.requires_holdings_authorisation());
    assert_eq!(h.requests().len(), 1, "a placement is never retried");
}

#[tokio::test]
async fn a_429_is_not_retried() {
    // Supplemental: the documented rate-limit refusal (kite:exceptions.md:39)
    // in the error envelope, before the official response.
    let h = HttpHarness::start(vec![
        Reply::Respond {
            status: 429,
            content_type: "application/json",
            body: br#"{"status":"error","message":"Too many requests","error_type":"NetworkException"}"#
                .to_vec(),
        },
        Reply::json(fixtures::json_body("holdings_auth.json").unwrap()),
    ])
    .await;
    let err = client(&h.base_url())
        .portfolio()
        .authorise_holdings(&two_isins())
        .await
        .unwrap_err();
    assert_eq!(err.as_http().unwrap().http_status(), Some(429));
    assert_eq!(h.requests().len(), 1, "a mutation makes one attempt");
}

#[test]
fn the_portal_url_cannot_be_redirected_by_a_broker_value() {
    let auth = HoldingsAuthorisation {
        request_id: "a/../b?c=d".to_string(),
    };
    assert_eq!(
        auth.portal_url(&ApiKey::new("key").unwrap()),
        "https://kite.zerodha.com/connect/portfolio/authorise/holdings/key/a%2F..%2Fb%3Fc%3Dd"
    );
}

#[test]
fn authorisation_is_a_standard_class_mutation() {
    assert_eq!(
        RateClass::of(Method::Post, Endpoint::HoldingsAuthorise),
        RateClass::Standard
    );
    assert_eq!(
        RetryClass::of(Method::Post, Endpoint::HoldingsAuthorise),
        RetryClass::Mut
    );
    assert_eq!(
        Endpoint::HoldingsAuthorise.as_str(),
        "/portfolio/holdings/authorise"
    );
}
