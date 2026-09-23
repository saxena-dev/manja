//! Quote completeness and the instrument master (plan task S11).
//!
//! Baselines are the official `quote.json`, `ohlc.json`, `ltp.json`,
//! `instruments_all.csv` and `instruments_nse.csv`, served unchanged.
//! Limits come from `kite-api-docs/docs/connect/v3/market-quotes.md:272-278`.
//! Missing-key, unexpected-key and malformed variants are labelled
//! supplements derived from those files.

mod support;

use chrono::DateTime;
use manja::kite::connect::api::Market;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::{Exchange, FullQuote, InstrumentType, LTPQuote, OHLCQuote};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpErrorKind, ManjaError, TransportStage};
use manja::kite::protocol::{Inbound, InstrumentToken};

use support::fixtures;
use support::http::{HttpHarness, RecordedRequest, Reply};

fn client_with(base: &str, limits: HttpLimits) -> HTTPClient {
    let config = Config::new(base).with_limits(
        limits.with_scheduler(SchedulerLimits::default().with_read_attempts(1).unwrap()),
    );
    HTTPClient::with_config(config)
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

fn client(base: &str) -> HTTPClient {
    client_with(base, HttpLimits::default())
}

async fn serve(body: impl Into<Vec<u8>>) -> HttpHarness {
    HttpHarness::start(vec![Reply::json(body)]).await
}

fn only(h: &HttpHarness) -> RecordedRequest {
    let [r] = h.requests().try_into().unwrap();
    r
}

fn is_validation(e: &ManjaError) -> bool {
    let h = e.as_http().unwrap();
    h.kind() == HttpErrorKind::Validation && h.stage() == TransportStage::NotStarted
}

#[tokio::test]
async fn full_quote() {
    let h = serve(fixtures::json_body("quote.json").unwrap()).await;
    let c = client(&h.base_url());
    let quotes = Market::new(&c)
        .get_quotes::<FullQuote>(&["NSE:INFY"])
        .await
        .unwrap()
        .data
        .unwrap();
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("GET", "/quote?i=NSE%3AINFY")
    );
    assert!(quotes.is_complete());
    let q = quotes.get("NSE:INFY").unwrap();
    assert_eq!(q.instrument_token, InstrumentToken::new(408065));
    assert_eq!(
        q.timestamp,
        Some(DateTime::parse_from_rfc3339("2021-06-08T15:45:56+05:30").unwrap())
    );
    assert_eq!(q.last_price, 1412.95);
    assert_eq!(q.volume, Some(7360198));
    assert_eq!(q.ohlc.open, 1396.0);
    let depth = q.depth.as_ref().unwrap();
    assert_eq!((depth.buy.len(), depth.sell.len()), (5, 5));
    assert_eq!(depth.sell[0].price, 1412.95);
    assert_eq!(depth.sell[0].quantity, 5191);
    assert_eq!(depth.sell[0].orders, 13);
}

#[tokio::test]
async fn ohlc_and_ltp_quotes() {
    let h = serve(fixtures::json_body("ohlc.json").unwrap()).await;
    let c = client(&h.base_url());
    let quotes = Market::new(&c)
        .get_quotes::<OHLCQuote>(&["NSE:INFY"])
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(only(&h).target, "/quote/ohlc?i=NSE%3AINFY");
    assert_eq!(quotes.get("NSE:INFY").unwrap().last_price, 1075.0);

    let h = serve(fixtures::json_body("ltp.json").unwrap()).await;
    let c = client(&h.base_url());
    let quotes = Market::new(&c)
        .get_quotes::<LTPQuote>(&["NSE:INFY"])
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(only(&h).target, "/quote/ltp?i=NSE%3AINFY");
    assert_eq!(quotes.get("NSE:INFY").unwrap().last_price, 1074.35);
}

#[tokio::test]
async fn every_key_is_sent_and_missing_keys_are_reported_not_zeroed() {
    // The official ltp.json answers only NSE:INFY, so BSE:SENSEX is missing.
    let h = serve(fixtures::json_body("ltp.json").unwrap()).await;
    let c = client(&h.base_url());
    let quotes = Market::new(&c)
        .get_quotes::<LTPQuote>(&["NSE:INFY", "BSE:SENSEX"])
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(only(&h).target, "/quote/ltp?i=NSE%3AINFY&i=BSE%3ASENSEX");
    assert_eq!(quotes.requested, ["NSE:INFY", "BSE:SENSEX"]);
    assert_eq!(quotes.missing, ["BSE:SENSEX"]);
    assert!(quotes.get("BSE:SENSEX").is_none());
    assert!(!quotes.is_complete());
    assert!(quotes.unexpected.is_empty());
}

#[tokio::test]
async fn unrequested_keys_are_reported_as_unexpected() {
    // Supplemental, derived from ltp.json: the key renamed to one not asked for.
    let body = fixtures::json_body("ltp.json")
        .unwrap()
        .replace("NSE:INFY", "NSE:TCS");
    let h = serve(body).await;
    let c = client(&h.base_url());
    let quotes = Market::new(&c)
        .get_quotes::<LTPQuote>(&["NSE:INFY"])
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(quotes.missing, ["NSE:INFY"]);
    assert_eq!(quotes.unexpected, ["NSE:TCS"]);
}

#[tokio::test]
async fn limits_are_enforced_by_explicit_rejection() {
    let h = serve(fixtures::json_body("ltp.json").unwrap()).await;
    let c = client(&h.base_url());
    let m = Market::new(&c);
    let keys: Vec<String> = (0..1001).map(|i| format!("NSE:S{i}")).collect();
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    assert!(is_validation(
        &m.get_quotes::<FullQuote>(&refs[..501]).await.unwrap_err()
    ));
    assert!(is_validation(
        &m.get_quotes::<OHLCQuote>(&refs[..1001]).await.unwrap_err()
    ));
    assert!(is_validation(
        &m.get_quotes::<LTPQuote>(&refs[..1001]).await.unwrap_err()
    ));
    assert!(is_validation(
        &m.get_quotes::<LTPQuote>(&[]).await.unwrap_err()
    ));
    assert!(is_validation(
        &m.get_quotes::<LTPQuote>(&["NSE:INFY", "NSE:INFY"])
            .await
            .unwrap_err()
    ));
    for bad in ["INFY", ":INFY", "NSE:", ""] {
        assert!(is_validation(
            &m.get_quotes::<LTPQuote>(&[bad]).await.unwrap_err()
        ));
    }
    assert!(h.requests().is_empty(), "nothing was sent or truncated");
    // At the limit, every key goes out.
    let quotes = m
        .get_quotes::<LTPQuote>(&refs[..1000])
        .await
        .unwrap()
        .data
        .unwrap();
    let r = only(&h);
    assert_eq!(r.target.matches("i=").count(), 1000);
    assert_eq!(quotes.missing.len(), 1000);
    assert_eq!(quotes.unexpected, ["NSE:INFY"]);
}

#[tokio::test]
async fn a_malformed_quote_value_is_a_decode_error() {
    // Supplemental, derived from ltp.json: last_price as a string.
    let body = fixtures::json_body("ltp.json")
        .unwrap()
        .replace("1074.35", "\"n/a\"");
    let h = serve(body).await;
    let c = client(&h.base_url());
    let err = Market::new(&c)
        .get_quotes::<LTPQuote>(&["NSE:INFY"])
        .await
        .unwrap_err();
    assert_eq!(err.as_http().unwrap().kind(), HttpErrorKind::Decode);
}

#[tokio::test]
async fn instrument_master_is_parsed_with_documented_fields() {
    let csv = fixtures::read("instruments_all.csv").unwrap();
    let h = HttpHarness::start(vec![Reply::Respond {
        status: 200,
        content_type: "text/csv",
        body: csv.into_bytes(),
    }])
    .await;
    let c = client(&h.base_url());
    let all = Market::new(&c).get_instruments_all().await.unwrap();
    assert_eq!(only(&h).target, "/instruments");
    assert_eq!(all.len(), 99);
    let first = &all[0];
    assert_eq!(first.instrument_token, InstrumentToken::new(3813889));
    assert_eq!(first.exchange_token, "14898");
    assert_eq!(first.tradingsymbol, "CENTRALBK-BE");
    assert_eq!(first.name.as_deref(), Some("CENTRAL BANK OF INDIA"));
    assert_eq!(first.expiry, None);
    assert_eq!(first.tick_size, 0.05);
    assert_eq!(first.lot_size, 1);
    assert_eq!(
        first.instrument_type,
        Inbound::Known(InstrumentType::Equity)
    );
    assert_eq!(first.segment, "NSE");
    assert_eq!(first.exchange, Inbound::Known(Exchange::NSE));
    assert_eq!(first.quote_key(), "NSE:CENTRALBK-BE");
    let option = all.iter().find(|i| i.segment == "NFO-OPT").unwrap();
    assert!(option.expiry.is_some());
    assert!(matches!(
        option.instrument_type,
        Inbound::Known(InstrumentType::CallOption | InstrumentType::PutOption)
    ));
}

#[tokio::test]
async fn instruments_of_one_exchange() {
    let csv = fixtures::read("instruments_nse.csv").unwrap();
    let h = HttpHarness::start(vec![Reply::Respond {
        status: 200,
        content_type: "text/csv",
        body: csv.into_bytes(),
    }])
    .await;
    let c = client(&h.base_url());
    let nse = Market::new(&c)
        .get_instruments(Exchange::NSE)
        .await
        .unwrap();
    assert_eq!(only(&h).target, "/instruments/NSE");
    assert!(!nse.is_empty());
}

#[tokio::test]
async fn a_malformed_csv_row_fails_with_its_row_number() {
    // Supplemental, derived from instruments_all.csv: row 3's token made
    // non-numeric.
    let csv = fixtures::read("instruments_all.csv")
        .unwrap()
        .replacen("4645121,", "abc,", 1);
    let h = HttpHarness::start(vec![Reply::Respond {
        status: 200,
        content_type: "text/csv",
        body: csv.into_bytes(),
    }])
    .await;
    let c = client(&h.base_url());
    let err = Market::new(&c).get_instruments_all().await.unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Decode);
    assert!(e.detail().unwrap().as_str().contains("row 2"), "{e}");
}

#[tokio::test]
async fn an_oversized_instrument_dump_is_bounded() {
    let limits = HttpLimits::default().with_csv_body_bytes(1 << 20).unwrap();
    let mut csv = fixtures::read("instruments_all.csv").unwrap();
    while csv.len() <= (1 << 20) {
        csv.push_str(
            "3813889,14898,CENTRALBK-BE,CENTRAL BANK OF INDIA,0.0,,0.0,0.05,1,EQ,NSE,NSE\n",
        );
    }
    let h = HttpHarness::start(vec![Reply::Respond {
        status: 200,
        content_type: "text/csv",
        body: csv.into_bytes(),
    }])
    .await;
    let c = client_with(&h.base_url(), limits);
    let err = Market::new(&c).get_instruments_all().await.unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Decode);
    assert_eq!(e.http_status(), Some(200));
}
