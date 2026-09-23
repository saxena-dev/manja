//! Read response DTOs (plan task S08) against the official
//! `kiteconnect-mocks/` fixtures, served unchanged through the loopback
//! harness. Expected values are written out from the fixtures by hand, never
//! obtained by parsing the fixture a second time. Supplemental variants are
//! labelled with the official body they were derived from.

mod support;

use chrono::{DateTime, FixedOffset};
use manja::kite::connect::api::Portfolio;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::{Credentials, KiteCredentials};
use manja::kite::connect::models::{
    Exchange, Order, OrderStatus, OrderType, OrderValidity, OrderVariety, ProductType, SegmentKind,
    TransactionType,
};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::HttpErrorKind;
use manja::kite::protocol::{BrokerTimestamp, Inbound, InstrumentToken};

use support::fixtures;
use support::http::{HttpHarness, RecordedRequest, Reply};

fn client(base: &str) -> HTTPClient {
    let config = Config::from_parts(base, base, base, KiteCredentials::new("", "", "", "", ""))
        .with_limits(
            HttpLimits::default()
                .with_scheduler(SchedulerLimits::default().with_read_attempts(1).unwrap()),
        );
    HTTPClient::with_config(config)
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

async fn serve(fixture: &str) -> HttpHarness {
    HttpHarness::start(vec![Reply::json(fixtures::json_body(fixture).unwrap())]).await
}

fn only_request(h: &HttpHarness) -> RecordedRequest {
    let [r] = h.requests().try_into().unwrap();
    r
}

fn assert_get(r: &RecordedRequest, target: &str) {
    assert_eq!((r.method.as_str(), r.target.as_str()), ("GET", target));
    assert_eq!(r.header("x-kite-version"), Some("3"));
    assert_eq!(
        r.header("authorization"),
        Some("token test_api_key:test_access_token")
    );
    assert!(r.body.is_empty());
}

fn ist(s: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(s).unwrap()
}

#[tokio::test]
async fn profile() {
    let h = serve("profile.json").await;
    let p = client(&h.base_url())
        .user()
        .profile()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_get(&only_request(&h), "/user/profile");
    assert_eq!(p.user_id, "AB1234");
    assert_eq!(p.user_type, "individual");
    assert_eq!(p.email, "xxxyyy@gmail.com");
    assert_eq!(p.user_shortname, "AxAx");
    assert_eq!(p.exchanges.len(), 8);
    assert!(p.exchanges.contains(&"MF".to_string()));
    assert_eq!(p.products, ["CNC", "NRML", "MIS", "BO", "CO"]);
    assert_eq!(p.order_types, ["MARKET", "LIMIT", "SL", "SL-M"]);
    assert_eq!(p.avatar_url, None);
    assert_eq!(p.meta.demat_consent, "physical");
}

#[tokio::test]
async fn margins_all_and_by_segment() {
    let h = serve("margins.json").await;
    let m = client(&h.base_url())
        .user()
        .margins()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_get(&only_request(&h), "/user/margins");
    let equity = m.equity.unwrap();
    assert!(equity.enabled);
    assert_eq!(equity.available.cash, 245431.6);
    assert_eq!(equity.available.opening_balance, 245431.6);
    assert_eq!(equity.utilised.debits, 145706.55);
    assert_eq!(equity.utilised.exposure, 38981.25);
    assert!(m.commodity.is_some());

    let h = serve("margins_equity.json").await;
    client(&h.base_url())
        .user()
        .margins_by_segment(SegmentKind::Equity)
        .await
        .unwrap();
    assert_get(&only_request(&h), "/user/margins/equity");

    let h = serve("margin_commodity.json").await;
    let seg = client(&h.base_url())
        .user()
        .margins_by_segment(SegmentKind::Commodity)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_get(&only_request(&h), "/user/margins/commodity");
    assert!(seg.enabled);
}

#[tokio::test]
async fn order_book_parses_documented_timestamps() {
    // Regression (arch §2, finding F12): records with valid offset-free
    // timestamps used to fail deserialization entirely.
    let h = serve("orders.json").await;
    let mut c = client(&h.base_url());
    let orders = c.orders().list_orders().await.unwrap().data.unwrap();
    assert_get(&only_request(&h), "/orders");
    assert_eq!(orders.len(), 10);
    let o = &orders[0];
    assert_eq!(o.order_id, "100000000000000");
    assert_eq!(o.exchange_order_id.as_deref(), Some("200000000000000"));
    assert_eq!(o.status, Inbound::Known(OrderStatus::Cancelled));
    assert_eq!(o.variety, Inbound::Known(OrderVariety::Regular));
    assert_eq!(o.exchange, Inbound::Known(Exchange::CDS));
    assert_eq!(o.tradingsymbol, "USDINR21JUNFUT");
    assert_eq!(o.instrument_token, InstrumentToken::new(412675));
    assert_eq!(o.order_type, Inbound::Known(OrderType::Limit));
    assert_eq!(o.transaction_type, Inbound::Known(TransactionType::BUY));
    assert_eq!(o.validity, Inbound::Known(OrderValidity::Day));
    assert_eq!(o.product, Inbound::Known(ProductType::Normal));
    assert_eq!((o.quantity, o.price, o.cancelled_quantity), (1, 72.0, 1));
    assert_eq!(o.order_timestamp, Some(ist("2021-05-31T09:18:57+05:30")));
    assert_eq!(o.exchange_timestamp, Some(ist("2021-05-31T09:15:38+05:30")));
    assert_eq!(o.modified, Some(false));
    // Nulls stay null.
    let rejected = &orders[3];
    assert_eq!(rejected.status, Inbound::Known(OrderStatus::Rejected));
    assert_eq!(rejected.exchange_order_id, None);
    assert_eq!(rejected.exchange_timestamp, None);
    assert_eq!(rejected.validity, Inbound::Known(OrderValidity::TimeToLive));
    assert_eq!(rejected.validity_ttl, Some(2));
    assert!(orders
        .iter()
        .any(|o| o.variety == Inbound::Known(OrderVariety::Auction)));
    assert!(orders
        .iter()
        .any(|o| o.product == Inbound::Known(ProductType::MarginTradingFacility)));
}

#[tokio::test]
async fn order_history_preserves_an_undocumented_status() {
    let h = serve("order_info.json").await;
    let mut c = client(&h.base_url());
    let history = c
        .orders()
        .get_order_history("171229000724687")
        .await
        .unwrap()
        .data
        .unwrap();
    assert_get(&only_request(&h), "/orders/171229000724687");
    assert_eq!(history.len(), 8);
    assert_eq!(
        history[0].status,
        Inbound::Known(OrderStatus::PutOrderReqReceived)
    );
    // The official fixture carries "MODIFIED", which no documented status
    // table lists: it is preserved, and it cannot become an outbound value.
    let modified = history
        .iter()
        .find(|o| o.status.is_unknown())
        .expect("the MODIFIED record");
    assert_eq!(modified.status.as_wire(), "MODIFIED");
    assert!(OrderStatus::try_from(modified.status.clone()).is_err());
    assert_eq!(history[0].exchange_timestamp, None);
    // Fields absent from this endpoint's records stay absent.
    assert_eq!(history[0].meta, None);
    assert_eq!(history[0].guid, None);
}

#[tokio::test]
async fn trade_book_and_order_trades() {
    let h = serve("trades.json").await;
    let mut c = client(&h.base_url());
    let trades = c.orders().list_trades().await.unwrap().data.unwrap();
    assert_get(&only_request(&h), "/trades");
    assert_eq!(trades.len(), 4);
    let t = &trades[0];
    assert_eq!(t.trade_id, "10000000");
    assert_eq!(t.instrument_token, InstrumentToken::new(779521));
    assert_eq!(t.average_price, 420.65);
    assert_eq!(t.fill_timestamp, Some(ist("2021-05-31T09:16:39+05:30")));
    // The trade book's order_timestamp is a bare time of day.
    assert!(matches!(
        t.order_timestamp,
        Some(BrokerTimestamp::TimeOfDay(_))
    ));

    let h = serve("order_trades.json").await;
    let mut c = client(&h.base_url());
    let trades = c
        .orders()
        .get_order_trades("200000000000000")
        .await
        .unwrap()
        .data
        .unwrap();
    assert_get(&only_request(&h), "/orders/200000000000000/trades");
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].exchange, Inbound::Known(Exchange::MCX));
}

#[tokio::test]
async fn holdings() {
    let h = serve("holdings.json").await;
    let c = client(&h.base_url());
    let holdings = Portfolio::new(&c)
        .get_holdings()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_get(&only_request(&h), "/portfolio/holdings");
    assert_eq!(holdings.len(), 2);
    let a = &holdings[0];
    assert_eq!(a.tradingsymbol, "AARON");
    assert_eq!(a.isin, "INE721Z01010");
    assert_eq!(a.authorised_date, Some(ist("2025-01-17T00:00:00+05:30")));
    assert_eq!(a.collateral_type.as_deref(), Some(""));
    assert_eq!(a.last_price, 352.95);
    assert_eq!(a.mtf.as_ref().unwrap().quantity, 1000);
    assert_eq!(a.short_quantity, Some(0));
}

#[tokio::test]
async fn positions_are_the_net_and_day_object() {
    let h = serve("positions.json").await;
    let c = client(&h.base_url());
    let p = Portfolio::new(&c)
        .get_positions()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_get(&only_request(&h), "/portfolio/positions");
    assert!(!p.net.is_empty() && !p.day.is_empty());
    let first = &p.net[0];
    assert_eq!(first.tradingsymbol, "LEADMINI17DECFUT");
    assert_eq!(first.exchange, Inbound::Known(Exchange::MCX));
    assert_eq!(first.multiplier, 1000);
    assert_eq!(first.value, -161050.0);
    // The fixture's "CO" product is undocumented in orders.md and preserved.
    assert!(p
        .net
        .iter()
        .chain(&p.day)
        .any(|x| x.product.as_wire() == "CO" && x.product.is_unknown()));
}

#[tokio::test]
async fn holdings_auctions() {
    let h = serve("auctions_list.json").await;
    let c = client(&h.base_url());
    let auctions = Portfolio::new(&c)
        .get_auctions()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_get(&only_request(&h), "/portfolio/holdings/auctions");
    assert_eq!(auctions.len(), 3);
    assert_eq!(auctions[0].tradingsymbol, "ASHOKLEY");
    assert_eq!(auctions[0].auction_number, "20");
    assert_eq!(
        auctions[0].authorised_date,
        Some(ist("2022-12-21T00:00:00+05:30"))
    );
}

#[tokio::test]
async fn a_malformed_timestamp_fails_visibly() {
    // Supplemental, derived from orders.json: one order_timestamp rewritten
    // in a non-documented form.
    let body = fixtures::json_body("orders.json").unwrap().replacen(
        "2021-05-31 09:18:57",
        "31/05/2021 09:18",
        1,
    );
    let h = HttpHarness::start(vec![Reply::json(body)]).await;
    let mut c = client(&h.base_url());
    let err = c.orders().list_orders().await.unwrap_err();
    assert_eq!(err.as_http().unwrap().kind(), HttpErrorKind::Decode);
}

#[test]
fn every_optional_order_field_may_be_null() {
    // Supplemental, derived from orders.json record 0: each optional field
    // set to null in turn.
    let text = fixtures::read("orders.json").unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let record = doc["data"][0].clone();
    for field in [
        "parent_order_id",
        "exchange_order_id",
        "modified",
        "validity_ttl",
        "market_protection",
        "order_timestamp",
        "exchange_timestamp",
        "exchange_update_timestamp",
        "status_message",
        "status_message_raw",
        "meta",
        "tag",
        "guid",
    ] {
        let mut r = record.clone();
        r[field] = serde_json::Value::Null;
        let o: Order = serde_json::from_value(r).unwrap_or_else(|e| panic!("{field}: {e}"));
        assert_eq!(o.order_id, "100000000000000");
    }
}

#[test]
fn acknowledgements_carry_only_the_order_id() {
    let text = fixtures::read("order_response.json").unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let ack: manja::kite::connect::models::OrderReceipt =
        serde_json::from_value(doc["data"].clone()).unwrap();
    assert_eq!(ack.order_id, "151220000000000");
}
