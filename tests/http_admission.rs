//! Admission wired into the HTTP client (plan task S05): a request refused
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
