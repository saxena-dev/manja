//! Mutual fund orders, SIPs, holdings and instruments against the loopback
//! harness.
//!
//! Baselines are the official `mf_orders.json`, `mf_orders_info.json`,
//! `mf_sips.json`, `mf_holdings.json` and `mf_instruments.csv`, served
//! unchanged, and expected values are written out by hand from them. The
//! documentation lists only these reads (`kite:mutual-funds.md:5-11`); the
//! order and SIP mutation fixtures have no documented endpoint and are not
//! used.

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone};
use manja::kite::connect::admission::RateClass;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::{
    DividendType, MfOrderStatus, MfOrderVariety, MfPlan, MfPurchaseType, SchemeType, SipFrequency,
    SipStatus, TransactionType,
};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpErrorKind, TransportStage};
use manja::kite::obs::schema::{Endpoint, Method};
use manja::kite::protocol::Inbound;

use support::fixtures;
use support::http::{HttpHarness, RecordedRequest, Reply};

fn client(base: &str) -> HTTPClient {
    let scheduler = SchedulerLimits::default()
        .with_backoff(Duration::from_millis(10), Duration::from_millis(10))
        .unwrap()
        .with_jitter_seed(13);
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

fn ist(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(5 * 3600 + 30 * 60)
        .unwrap()
        .with_ymd_and_hms(y, mo, d, h, mi, s)
        .unwrap()
}

fn date(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(y, m, d)
}

#[tokio::test]
async fn the_order_list_decodes_every_documented_field() {
    let h = serve("mf_orders.json").await;
    let orders = client(&h.base_url())
        .mutual_funds()
        .list_orders()
        .await
        .unwrap()
        .data
        .unwrap();
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("GET", "/mf/orders")
    );
    assert_eq!(orders.len(), 5);

    let o = &orders[0];
    assert_eq!(o.order_id, "271989e0-a64e-4cf3-b4e4-afb8f38dd203");
    assert_eq!(o.exchange_order_id.as_deref(), Some("254657127"));
    assert_eq!(o.tradingsymbol, "INF179K01VY8");
    assert_eq!(o.status, Some(Inbound::Known(MfOrderStatus::Rejected)));
    assert_eq!(
        o.status_message.as_deref(),
        Some("AMC SIP: Insufficient balance.")
    );
    assert_eq!(o.folio, None);
    assert_eq!(o.fund, "HDFC Balanced Advantage Fund - Direct Plan");
    assert_eq!(o.order_timestamp, Some(ist(2021, 6, 30, 8, 33, 7)));
    assert_eq!(o.exchange_timestamp, date(2021, 6, 30));
    assert_eq!(o.settlement_id.as_deref(), Some("2122061"));
    assert_eq!(o.transaction_type, Inbound::Known(TransactionType::BUY));
    assert_eq!(o.amount, 1000.0);
    assert_eq!(o.variety, Inbound::Known(MfOrderVariety::AmcSip));
    assert_eq!(o.purchase_type, Some(Inbound::Known(MfPurchaseType::Fresh)));
    assert_eq!(
        (o.quantity, o.average_price, o.last_price),
        (0.0, 0.0, 30.68)
    );
    assert_eq!(o.price, None);
    assert_eq!(o.placed_by, "ZV8062");
    assert_eq!(o.last_price_date, date(2021, 6, 29));
    assert_eq!(o.tag, None);

    let o = &orders[1];
    assert_eq!(o.variety, Inbound::Known(MfOrderVariety::Sip));
    assert_eq!(
        o.purchase_type,
        Some(Inbound::Known(MfPurchaseType::Additional))
    );
    assert_eq!(o.exchange_order_id, None);
    assert_eq!(o.exchange_timestamp, None);
    assert_eq!(o.settlement_id, None);
    assert_eq!(o.tag.as_deref(), Some("coinandroidsip"));
    assert_eq!(o.amount, 2000.0);

    let o = &orders[2];
    assert_eq!(o.status, Some(Inbound::Known(MfOrderStatus::Open)));
    assert_eq!(o.variety, Inbound::Known(MfOrderVariety::Regular));
    let amounts: Vec<f64> = orders.iter().map(|o| o.amount).collect();
    assert_eq!(amounts, [1000.0, 2000.0, 5000.0, 1000.0, 5000.0]);
}

#[tokio::test]
async fn one_order_is_fetched_by_its_uuid() {
    let h = serve("mf_orders_info.json").await;
    let o = client(&h.base_url())
        .mutual_funds()
        .get_order("2b6ad4b7-c84e-4c76-b459-f3a8994184f1")
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(
        only(&h).target,
        "/mf/orders/2b6ad4b7-c84e-4c76-b459-f3a8994184f1"
    );
    assert_eq!(o.order_id, "2b6ad4b7-c84e-4c76-b459-f3a8994184f1");
    assert_eq!(o.status, Some(Inbound::Known(MfOrderStatus::Open)));
    assert_eq!(o.status_message.as_deref(), Some("Insufficient fund. 1/5"));
    // The sample sends the quantity as the integer 0.
    assert_eq!(o.quantity, 0.0);
    assert_eq!(o.fund, "BOI AXA Arbitrage Fund - Direct Plan");
}

#[tokio::test]
async fn an_order_id_that_could_change_the_path_sends_nothing() {
    let h = HttpHarness::start(vec![]).await;
    let c = client(&h.base_url());
    for bad in ["", "a/../b", "a?x=y", "has space"] {
        let err = c.mutual_funds().get_order(bad).await.unwrap_err();
        let e = err.as_http().unwrap();
        assert_eq!(e.kind(), HttpErrorKind::Validation, "{bad:?}");
        assert_eq!(e.stage(), TransportStage::NotStarted);
        assert_eq!(e.endpoint(), Endpoint::MfOrdersId);
    }
    assert!(h.requests().is_empty());
}

#[tokio::test]
async fn the_sip_list_decodes_every_documented_field() {
    let h = serve("mf_sips.json").await;
    let sips = client(&h.base_url())
        .mutual_funds()
        .list_sips()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(only(&h).target, "/mf/sips");
    assert_eq!(sips.len(), 5);

    let s = &sips[0];
    assert_eq!(s.sip_id, "892741486820670");
    assert_eq!(s.tradingsymbol, "INF209K01VD7");
    assert_eq!(s.fund, "Aditya Birla Sun Life Liquid Fund - Direct Plan");
    assert_eq!(s.dividend_type, Inbound::Known(DividendType::Idcw));
    assert_eq!(s.transaction_type, Inbound::Known(TransactionType::BUY));
    assert_eq!(s.status, Inbound::Known(SipStatus::Active));
    assert_eq!(s.created, Some(ist(2021, 5, 5, 5, 56, 27)));
    assert_eq!(s.frequency, Inbound::Known(SipFrequency::Weekly));
    assert_eq!(s.next_instalment, date(2021, 5, 12));
    assert_eq!(s.instalment_amount, 500.0);
    assert_eq!((s.instalments, s.pending_instalments), (-1, -1));
    assert!(s.is_open_ended());
    assert_eq!(s.last_instalment, Some(ist(2021, 5, 5, 5, 56, 27)));
    assert_eq!((s.instalment_day, s.completed_instalments), (0, 0));
    assert_eq!(s.tag.as_deref(), Some("coiniossip"));
    assert_eq!(s.sip_reg_num, None);
    assert_eq!(s.sip_type.as_deref(), Some("sip"));
    assert_eq!(s.trigger_price, Some(0.0));
    assert_eq!(
        s.step_up,
        Some(BTreeMap::from([("05-05".to_string(), 10.0)]))
    );

    let s = &sips[2];
    assert_eq!(s.frequency, Inbound::Known(SipFrequency::Monthly));
    assert_eq!((s.instalments, s.pending_instalments), (9999, 9998));
    assert!(!s.is_open_ended());
    assert_eq!((s.instalment_day, s.completed_instalments), (10, 1));
    assert_eq!(s.sip_reg_num.as_deref(), Some("15158182"));
    assert_eq!(s.sip_type.as_deref(), Some("amc_sip"));
    assert_eq!(s.step_up, Some(BTreeMap::new()));
    assert_eq!(s.last_instalment, Some(ist(2021, 6, 10, 8, 37, 11)));

    let s = &sips[4];
    assert_eq!(s.dividend_type, Inbound::Known(DividendType::Growth));
    assert_eq!(s.frequency, Inbound::Known(SipFrequency::Quarterly));
    assert_eq!(s.instalment_amount, 7427.0);
}

#[tokio::test]
async fn holdings_decode_and_an_empty_nav_date_is_none() {
    let h = serve("mf_holdings.json").await;
    let holdings = client(&h.base_url())
        .mutual_funds()
        .list_holdings()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(only(&h).target, "/mf/holdings");
    assert_eq!(holdings.len(), 3);
    let x = &holdings[0];
    assert_eq!(x.folio.as_deref(), Some("3108290884"));
    assert_eq!(x.fund, "INVESCO INDIA TAX PLAN - DIRECT PLAN");
    assert_eq!(x.tradingsymbol, "INF205K01NT8");
    assert_eq!((x.average_price, x.last_price, x.pnl), (78.43, 84.86, 0.0));
    assert_eq!(x.quantity, 382.488);
    assert_eq!(x.last_price_date, None);
    assert_eq!(x.pledged_quantity, Some(0.0));
    let quantities: Vec<f64> = holdings.iter().map(|x| x.quantity).collect();
    assert_eq!(quantities, [382.488, 1.334, 257.057]);
    assert_eq!(holdings[1].average_price, 1874.101138);
}

#[tokio::test]
async fn the_instrument_list_is_parsed_and_undocumented_values_preserved() {
    let csv = fixtures::read("mf_instruments.csv").unwrap();
    let h = HttpHarness::start(vec![Reply::Respond {
        status: 200,
        content_type: "text/csv",
        body: csv.into_bytes(),
    }])
    .await;
    let funds = client(&h.base_url())
        .mutual_funds()
        .get_instruments()
        .await
        .unwrap();
    assert_eq!(only(&h).target, "/mf/instruments");
    assert_eq!(funds.len(), 99);
    let f = &funds[0];
    assert_eq!(f.tradingsymbol, "INF209K01157");
    assert_eq!(f.amc, "BirlaSunLifeMutualFund_MF");
    assert_eq!(f.name, "Aditya Birla Sun Life Advantage Fund");
    assert!(f.purchase_allowed && f.redemption_allowed);
    assert_eq!(
        (
            f.minimum_purchase_amount,
            f.purchase_amount_multiplier,
            f.minimum_additional_purchase_amount,
            f.minimum_redemption_quantity,
            f.redemption_quantity_multiplier
        ),
        (1000.0, 1.0, 1000.0, 0.001, 0.001)
    );
    assert_eq!(f.dividend_type, Inbound::Known(DividendType::Payout));
    assert_eq!(f.scheme_type, Inbound::Known(SchemeType::Equity));
    assert_eq!(f.plan, Inbound::Known(MfPlan::Regular));
    assert_eq!(f.settlement_type, "T3");
    assert_eq!(f.last_price, 106.8);
    assert_eq!(f.last_price_date, date(2017, 11, 23));
    // The list carries scheme types the documentation does not name.
    let unknown: std::collections::BTreeSet<&str> = funds
        .iter()
        .filter(|f| f.scheme_type.is_unknown())
        .map(|f| f.scheme_type.as_wire())
        .collect();
    assert_eq!(
        unknown,
        std::collections::BTreeSet::from(["balanced", "fof", "liquid"])
    );
    assert_eq!(funds[98].tradingsymbol, "INF209K01VR7");
    assert_eq!(funds[98].plan, Inbound::Known(MfPlan::Direct));
}

#[tokio::test]
async fn the_raw_instrument_csv_is_returned_unchanged() {
    let csv = fixtures::read("mf_instruments.csv").unwrap();
    let h = HttpHarness::start(vec![Reply::Respond {
        status: 200,
        content_type: "text/csv",
        body: csv.clone().into_bytes(),
    }])
    .await;
    let raw = client(&h.base_url())
        .mutual_funds()
        .get_instruments_csv()
        .await
        .unwrap();
    assert_eq!(raw, csv);
}

#[tokio::test]
async fn a_malformed_instrument_row_is_a_decode_error_naming_it() {
    // Supplemental, derived from mf_instruments.csv: row 1's purchase flag
    // becomes "yes".
    let csv = fixtures::read("mf_instruments.csv").unwrap().replacen(
        "Aditya Birla Sun Life Advantage Fund,1,",
        "Aditya Birla Sun Life Advantage Fund,yes,",
        1,
    );
    let h = HttpHarness::start(vec![Reply::Respond {
        status: 200,
        content_type: "text/csv",
        body: csv.into_bytes(),
    }])
    .await;
    let err = client(&h.base_url())
        .mutual_funds()
        .get_instruments()
        .await
        .unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Decode);
    assert_eq!(e.endpoint(), Endpoint::MfInstruments);
    assert!(format!("{e}").contains("row 1"), "{e}");
}

#[tokio::test]
async fn an_unknown_status_is_preserved() {
    // Supplemental, derived from mf_orders_info.json: status "PROCESSING".
    let body = fixtures::json_body("mf_orders_info.json")
        .unwrap()
        .replace("\"status\": \"OPEN\"", "\"status\": \"PROCESSING\"");
    let h = HttpHarness::start(vec![Reply::json(body)]).await;
    let o = client(&h.base_url())
        .mutual_funds()
        .get_order("2b6ad4b7-c84e-4c76-b459-f3a8994184f1")
        .await
        .unwrap()
        .data
        .unwrap();
    let status = o.status.unwrap();
    assert!(status.is_unknown());
    assert_eq!(status.as_wire(), "PROCESSING");
}

#[tokio::test]
async fn reads_retry_a_transient_failure() {
    // Supplemental: a 503 before the official holdings.
    let h = HttpHarness::start(vec![
        Reply::Respond {
            status: 503,
            content_type: "text/html",
            body: b"<html>unavailable</html>".to_vec(),
        },
        Reply::json(fixtures::json_body("mf_holdings.json").unwrap()),
    ])
    .await;
    let holdings = client(&h.base_url())
        .mutual_funds()
        .list_holdings()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(holdings.len(), 3);
    assert_eq!(h.requests().len(), 2);
}

#[test]
fn mutual_fund_endpoints_use_the_standard_quota_class() {
    for e in [
        Endpoint::MfOrders,
        Endpoint::MfOrdersId,
        Endpoint::MfSips,
        Endpoint::MfHoldings,
        Endpoint::MfInstruments,
    ] {
        assert_eq!(RateClass::of(Method::Get, e), RateClass::Standard, "{e:?}");
    }
}
