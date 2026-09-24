//! A strict list rejection is a failed operation and attempt in their spans.
//! This is the only test in its binary: span capture is per thread, and a
//! test running without a subscriber could otherwise reach the span first
//! and have it cached as disabled.
//!
//! The body is supplemental, derived from the official `orders.json`: one
//! row's price made invalid.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::Credentials;
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

use support::fixtures;
use support::http::{HttpHarness, Reply};

type Spans = Arc<Mutex<Vec<(u64, &'static str, BTreeMap<String, String>)>>>;

#[derive(Clone, Default)]
struct Capture(Spans);

struct Fields<'a>(&'a mut BTreeMap<String, String>);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Capture {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        let mut fields = BTreeMap::new();
        attrs.record(&mut Fields(&mut fields));
        self.0
            .lock()
            .unwrap()
            .push((id.into_u64(), attrs.metadata().name(), fields));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut spans = self.0.lock().unwrap();
        if let Some(s) = spans.iter_mut().rev().find(|s| s.0 == id.into_u64()) {
            values.record(&mut Fields(&mut s.2));
        }
    }
}

#[tokio::test]
async fn a_strict_rejection_ends_its_spans_as_decode_errors() {
    let capture = Capture::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(capture.clone()));
    let mut v: Value = serde_json::from_str(&fixtures::json_body("orders.json").unwrap()).unwrap();
    v["data"][2]["price"] = Value::from("SEEDprice");
    let h = HttpHarness::start(vec![Reply::json(serde_json::to_string(&v).unwrap())]).await;
    let c = HTTPClient::with_config(Config::new(&*h.base_url()))
        .unwrap()
        .with_credentials(Credentials::new("k", "t").unwrap());
    c.orders().list_orders().await.unwrap_err();

    let spans = capture.0.lock().unwrap().clone();
    let result = |name: &str| {
        spans
            .iter()
            .find(|s| s.1 == name)
            .and_then(|s| s.2.get("result").cloned())
    };
    assert_eq!(
        result("manja.http.operation").as_deref(),
        Some("decode_error")
    );
    // A failed attempt's span records its result as `error_class`, beside
    // the 200 that carried the rejected row.
    let attempt = spans
        .iter()
        .find(|s| s.1 == "manja.http.attempt")
        .expect("one attempt span");
    assert_eq!(
        attempt.2.get("error_class").map(String::as_str),
        Some("decode_error")
    );
    assert_eq!(
        attempt.2.get("http_status").map(String::as_str),
        Some("200")
    );
    // The seeded value is in no span field.
    assert!(spans
        .iter()
        .all(|s| s.2.values().all(|v| !v.contains("SEEDprice"))));
}
