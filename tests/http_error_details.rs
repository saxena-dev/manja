//! No value from a response body reaches an `HttpError` detail
//! (`contract.md` §2.4), against the loopback harness.
//!
//! Every body is supplemental: an official fixture or envelope made
//! malformed or mistyped with a distinctive seeded string, number or
//! symbol. The detail must still say what failed and where, and neither
//! `detail()`, `Display` nor `Debug` may carry a seed.

mod support;

use manja::kite::connect::api::{Market, MutualFunds, User};
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::LTPQuote;
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpErrorKind, ManjaError};

use support::fixtures;
use support::http::{HttpHarness, Reply};

const SEEDS: [&str; 6] = [
    "SEEDbare",
    "SEEDcut",
    "918273645",
    "SEEDstring",
    "SEEDSYM",
    "SEEDcsv",
];

fn client(base: &str) -> HTTPClient {
    let limits = HttpLimits::default()
        .with_scheduler(SchedulerLimits::default().with_read_attempts(1).unwrap());
    HTTPClient::with_config(Config::new(base).with_limits(limits))
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

async fn serve(content_type: &'static str, body: String) -> HttpHarness {
    HttpHarness::start(vec![Reply::Respond {
        status: 200,
        content_type,
        body: body.into_bytes(),
    }])
    .await
}

/// A `Decode` error whose detail is `expected` and carries no seed in any
/// rendering.
fn assert_clean(err: &ManjaError, expected: &str) {
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Decode, "{err:?}");
    let detail = e.detail().unwrap().as_str();
    assert_eq!(detail, expected);
    for rendered in [
        detail.to_string(),
        e.to_string(),
        err.to_string(),
        format!("{e:?}"),
        format!("{err:?}"),
    ] {
        for seed in SEEDS {
            assert!(!rendered.contains(seed), "{seed} in {rendered}");
        }
    }
}

async fn profile_error(body: String) -> ManjaError {
    let h = serve("application/json", body).await;
    User::new(&client(&h.base_url()))
        .profile()
        .await
        .unwrap_err()
}

#[tokio::test]
async fn malformed_envelopes_name_the_position_not_the_text() {
    let err = profile_error(r#"{"status":"success","data":SEEDbare}"#.into()).await;
    assert_clean(
        &err,
        "malformed JSON success body: syntax error at line 1 column 28",
    );
    let err = profile_error(r#"{"status":"success","data":{"user_id":"SEEDcut"#.into()).await;
    assert_clean(
        &err,
        "malformed JSON success body: unexpected end of input at line 1 column 46",
    );
}

#[tokio::test]
async fn mistyped_payloads_name_the_category_not_the_value() {
    let mismatch = "the payload does not match the endpoint's type: data error";

    // A number where the profile has a string.
    let body = fixtures::json_body("profile.json")
        .unwrap()
        .replace("\"AB1234\"", "918273645");
    assert_clean(&profile_error(body).await, mismatch);

    // A string where a quote has a number, under a seeded symbol.
    let body = fixtures::json_body("ltp.json")
        .unwrap()
        .replace("NSE:INFY", "NSE:SEEDSYM")
        .replace("1074.35", "\"SEEDstring\"");
    let h = serve("application/json", body).await;
    let err = Market::new(&client(&h.base_url()))
        .get_quotes::<LTPQuote>(&["NSE:SEEDSYM"])
        .await
        .unwrap_err();
    assert_clean(&err, mismatch);
}

#[tokio::test]
async fn malformed_csv_rows_name_the_row_not_the_value() {
    let csv = fixtures::read("instruments_all.csv")
        .unwrap()
        .replacen("4645121,", "SEEDcsv,", 1);
    let h = serve("text/csv", csv).await;
    let err = Market::new(&client(&h.base_url()))
        .get_instruments_all()
        .await
        .unwrap_err();
    assert_clean(&err, "instrument CSV row 2 is malformed");

    let csv = fixtures::read("mf_instruments.csv")
        .unwrap()
        .replacen("1000.0", "SEEDcsv", 1);
    let h = serve("text/csv", csv).await;
    let err = MutualFunds::new(&client(&h.base_url()))
        .get_instruments()
        .await
        .unwrap_err();
    assert_clean(&err, "mutual fund instrument CSV row 1 is malformed");
}
