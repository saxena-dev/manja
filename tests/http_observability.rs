//! HTTP operation, attempt and admission instrumentation.
//!
//! Spans are captured by a test-local `tracing` layer installed as the
//! thread's default subscriber; metrics by the SDK's `InMemoryRecorder`.
//! Nothing global is installed. Baselines are official fixtures served
//! unchanged; faults are labelled supplements.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use manja::kite::connect::admission::{Admission, AdmissionLimits, QuotaProfile};
use manja::kite::connect::api::Market;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::{ApiKey, ApiSecret, Credentials, RequestToken};
use manja::kite::connect::models::{LTPQuote, OrderVariety};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::obs::{BridgeRecorder, InMemoryRecorder, Instrument, Observability};
use manja::kite::protocol::OrderId;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

use support::fixtures;
use support::http::{HttpHarness, Reply};

// ---- span capture -------------------------------------------------------

#[derive(Clone, Debug)]
struct SpanRec {
    id: u64,
    name: &'static str,
    parent: Option<u64>,
    fields: BTreeMap<String, String>,
}

impl SpanRec {
    fn field(&self, k: &str) -> Option<&str> {
        self.fields.get(k).map(String::as_str)
    }
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<SpanRec>>>);

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
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let parent = ctx
            .span(id)
            .and_then(|s| s.parent())
            .map(|p| p.id().into_u64());
        let mut fields = BTreeMap::new();
        attrs.record(&mut Fields(&mut fields));
        self.0.lock().unwrap().push(SpanRec {
            id: id.into_u64(),
            name: attrs.metadata().name(),
            parent,
            fields,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut spans = self.0.lock().unwrap();
        // Span ids are reused after close; the latest one is live.
        if let Some(s) = spans.iter_mut().rev().find(|s| s.id == id.into_u64()) {
            values.record(&mut Fields(&mut s.fields));
        }
    }
}

impl Capture {
    fn install(&self) -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(tracing_subscriber::registry().with(self.clone()))
    }

    fn spans(&self) -> Vec<SpanRec> {
        self.0.lock().unwrap().clone()
    }

    fn named(&self, name: &str) -> Vec<SpanRec> {
        self.spans()
            .into_iter()
            .filter(|s| s.name == name)
            .collect()
    }
}

// ---- helpers ------------------------------------------------------------

const OP: &str = "manja.http.operation";
const ATTEMPT: &str = "manja.http.attempt";
const ADMISSION: &str = "manja.http.admission";

fn config(base: &str, scheduler: SchedulerLimits) -> Config {
    Config::new(base).with_limits(HttpLimits::default().with_scheduler(scheduler))
}

fn fast() -> SchedulerLimits {
    SchedulerLimits::default()
        .with_backoff(Duration::from_millis(10), Duration::from_millis(10))
        .unwrap()
        .with_jitter_seed(3)
}

fn client(base: &str, obs: &Observability) -> HTTPClient {
    HTTPClient::with_observability(config(base, fast()), obs.clone())
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

fn recording() -> (Arc<InMemoryRecorder>, Observability) {
    let rec = Arc::new(InMemoryRecorder::new());
    let obs = Observability::with_recorder(rec.clone());
    (rec, obs)
}

fn server_error() -> Reply {
    Reply::Respond {
        status: 503,
        content_type: "text/html",
        body: b"<html>unavailable</html>".to_vec(),
    }
}

fn gauges_settled(obs: &Observability) -> bool {
    obs.collect_gauges().iter().all(|g| g.value == 0.0)
}

// ---- operations and attempts --------------------------------------------

#[tokio::test]
async fn one_operation_span_and_count_with_one_child_per_actual_attempt() {
    let cap = Capture::default();
    let _g = cap.install();
    let (rec, obs) = recording();
    let h = HttpHarness::start(vec![
        server_error(),
        Reply::json(fixtures::json_body("orders.json").unwrap()),
    ])
    .await;
    let c = client(&h.base_url(), &obs);
    c.clone().orders().list_orders().await.unwrap();
    assert_eq!(h.requests().len(), 2);

    let ops = cap.named(OP);
    let [op] = ops.as_slice() else {
        panic!("{ops:?}")
    };
    assert_eq!(op.field("endpoint"), Some("/orders"));
    assert_eq!(op.field("quota_class"), Some("read"));
    assert_eq!(op.field("result"), Some("ok"));
    assert_eq!(op.field("stage"), Some("response_received"));
    assert!(op.field("deadline_ms").is_some());
    let attempts = cap.named(ATTEMPT);
    assert_eq!(attempts.len(), 2);
    for (i, a) in attempts.iter().enumerate() {
        assert_eq!(a.parent, Some(op.id), "attempts are children");
        assert_eq!(a.field("operation_id"), op.field("operation_id"));
        assert_eq!(a.field("attempt"), Some((i + 1).to_string().as_str()));
    }
    assert_eq!(attempts[0].field("http_status"), Some("503"));
    assert_eq!(attempts[0].field("error_class"), Some("http_status"));
    assert_eq!(attempts[1].field("http_status"), Some("200"));
    assert_eq!(attempts[1].field("error_class"), None);
    for a in cap.named(ADMISSION) {
        assert_eq!(a.parent, Some(op.id));
        assert_eq!(a.field("admission_result"), Some("granted"));
    }

    let op_labels = ["GET", "/orders", "read", "ok"];
    assert_eq!(rec.counter(Instrument::HttpOperationsTotal, &op_labels), 1);
    assert_eq!(rec.counter_total(Instrument::HttpOperationsTotal), 1);
    assert_eq!(
        rec.histogram(Instrument::HttpOperationDuration, &op_labels)
            .0,
        1
    );
    assert_eq!(
        rec.counter(
            Instrument::HttpAttemptsTotal,
            &["GET", "/orders", "http_status"]
        ),
        1
    );
    assert_eq!(
        rec.counter(Instrument::HttpAttemptsTotal, &["GET", "/orders", "ok"]),
        1
    );
    assert_eq!(
        rec.counter(
            Instrument::HttpRetriesTotal,
            &["GET", "/orders", "http_5xx"]
        ),
        1
    );
    assert_eq!(rec.counter_total(Instrument::HttpRetriesTotal), 1);
    assert!(gauges_settled(&obs));
}

#[tokio::test]
async fn pre_dispatch_failures_and_unpolled_futures_make_no_attempt() {
    let cap = Capture::default();
    let _g = cap.install();
    let (rec, obs) = recording();
    let h = HttpHarness::start(vec![Reply::json(fixtures::json_body("ltp.json").unwrap())]).await;
    let c = client(&h.base_url(), &obs);

    // Unpolled: nothing at all.
    let user = c.user();
    let unpolled = user.profile();
    drop(unpolled);
    assert!(cap.spans().is_empty());
    assert_eq!(rec.counter_total(Instrument::HttpOperationsTotal), 0);

    // Validation: one operation, no attempt.
    Market::new(&c)
        .get_quotes::<LTPQuote>(&[])
        .await
        .unwrap_err();
    assert_eq!(
        rec.counter(
            Instrument::HttpOperationsTotal,
            &["GET", "/quote/ltp", "read", "validation"]
        ),
        1
    );
    let ops = cap.named(OP);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].field("result"), Some("validation"));
    assert_eq!(ops[0].field("stage"), Some("not_started"));

    // Admission rejection: a second quote within the 1/s window and a
    // 1 ms admission wait bound. One operation, one admission wait, no
    // attempt.
    let admission = Admission::new(
        QuotaProfile::kite_v3(),
        AdmissionLimits::default()
            .with_wait(Duration::from_millis(1))
            .unwrap(),
    );
    let tight = HTTPClient::builder(config(&h.base_url(), fast()))
        .admission(admission)
        .observability(obs.clone())
        .credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
        .build()
        .unwrap();
    let m = Market::new(&tight);
    m.get_quotes::<LTPQuote>(&["NSE:INFY"]).await.unwrap();
    m.get_quotes::<LTPQuote>(&["NSE:INFY"]).await.unwrap_err();
    assert_eq!(h.requests().len(), 1);
    assert_eq!(
        rec.counter(
            Instrument::HttpOperationsTotal,
            &["GET", "/quote/ltp", "read", "admission_rejected"]
        ),
        1
    );
    assert_eq!(rec.counter_total(Instrument::HttpAttemptsTotal), 1);
    assert_eq!(cap.named(ATTEMPT).len(), 1);
    assert_eq!(
        rec.histogram(Instrument::HttpAdmissionWait, &["read", "rejected"])
            .0,
        1
    );
    assert_eq!(rec.counter_total(Instrument::HttpRetriesTotal), 0);
    assert!(gauges_settled(&obs));
}

#[tokio::test]
async fn mutations_never_count_a_retry() {
    let (rec, obs) = recording();
    let h = HttpHarness::start(vec![server_error(), server_error()]).await;
    let c = client(&h.base_url(), &obs);
    c.clone()
        .orders()
        .cancel_order(
            OrderVariety::Regular,
            &OrderId::new("151220000000000").unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(h.requests().len(), 1);
    assert_eq!(rec.counter_total(Instrument::HttpAttemptsTotal), 1);
    assert_eq!(rec.counter_total(Instrument::HttpRetriesTotal), 0);
    assert_eq!(
        rec.counter(
            Instrument::HttpOperationsTotal,
            &[
                "DELETE",
                "/orders/{variety}/{order_id}",
                "mut",
                "http_status"
            ]
        ),
        1
    );
}

// ---- durations ----------------------------------------------------------

#[tokio::test]
async fn operation_time_includes_admission_and_attempt_time_excludes_it() {
    let (rec, obs) = recording();
    let ltp = fixtures::json_body("ltp.json").unwrap();
    let h = HttpHarness::start(vec![Reply::json(ltp.clone()), Reply::json(ltp)]).await;
    let c = client(&h.base_url(), &obs);
    let m = Market::new(&c);
    m.get_quotes::<LTPQuote>(&["NSE:INFY"]).await.unwrap();
    // The quote class admits 1 request/s, so this one waits about 1 s.
    m.get_quotes::<LTPQuote>(&["NSE:INFY"]).await.unwrap();

    let (ops, op_secs) = rec.histogram(
        Instrument::HttpOperationDuration,
        &["GET", "/quote/ltp", "read", "ok"],
    );
    let (attempts, attempt_secs) = rec.histogram(
        Instrument::HttpAttemptDuration,
        &["GET", "/quote/ltp", "ok"],
    );
    let (waits, wait_secs) = rec.histogram(Instrument::HttpAdmissionWait, &["read", "granted"]);
    assert_eq!((ops, attempts, waits), (2, 2, 2));
    assert!(wait_secs >= 0.9, "admission wait measured: {wait_secs}");
    assert!(
        attempt_secs < 0.5,
        "attempts exclude admission: {attempt_secs}"
    );
    assert!(
        op_secs >= wait_secs + attempt_secs - 0.01,
        "operation {op_secs} covers wait {wait_secs} + attempts {attempt_secs}"
    );
}

// ---- gauges -------------------------------------------------------------

#[tokio::test]
async fn gauges_settle_on_success_error_cancellation_and_teardown() {
    let (_rec, obs) = recording();
    let h = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("profile.json").unwrap()),
        Reply::DropAfterRequest,
        Reply::DropAfterRequest,
        Reply::DropAfterRequest,
        Reply::Stall,
    ])
    .await;
    let c = client(&h.base_url(), &obs);
    let clone = c.clone();

    c.user().profile().await.unwrap();
    assert!(gauges_settled(&obs));
    clone.user().profile().await.unwrap_err();
    assert!(gauges_settled(&obs));

    // Mid-attempt the in-flight gauge and active count are 1; cancelling
    // the caller returns both to zero.
    let task = tokio::spawn({
        let c = clone.clone();
        async move { c.user().profile().await }
    });
    while h.requests().len() < 5 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let in_flight = obs
        .collect_gauges()
        .into_iter()
        .find(|g| g.labels.instrument() == Instrument::HttpInFlight)
        .unwrap();
    assert_eq!(in_flight.labels.values(), ["read"]);
    assert_eq!(in_flight.value, 1.0);
    assert_eq!(c.diagnostics().active_attempts, 1);
    task.abort();
    let _ = task.await;
    assert!(gauges_settled(&obs));
    assert_eq!(c.diagnostics().active_attempts, 0);

    // A waiter cancelled mid-admission leaves no waiter behind.
    let m = Market::new(&c);
    let _ = m.get_quotes::<LTPQuote>(&["NSE:INFY"]).await;
    let waiting = tokio::spawn({
        let c = clone.clone();
        async move { Market::new(&c).get_quotes::<LTPQuote>(&["NSE:INFY"]).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let by_class = c.diagnostics().admission_waiters_by_class;
    assert!(
        by_class.contains(&(manja::kite::obs::schema::QuotaClass::Read, 1)),
        "{by_class:?}"
    );
    assert_eq!(
        obs.collect_gauges()
            .iter()
            .filter(|g| g.labels.instrument() == Instrument::HttpAdmissionWaiters)
            .map(|g| g.value)
            .sum::<f64>(),
        1.0
    );
    waiting.abort();
    let _ = waiting.await;
    assert!(gauges_settled(&obs));
    assert!(c
        .diagnostics()
        .admission_waiters_by_class
        .iter()
        .all(|(_, n)| *n == 0));

    drop((c, clone));
    assert!(gauges_settled(&obs));
}

// ---- caller context -----------------------------------------------------

#[tokio::test]
async fn concurrent_callers_keep_their_own_context() {
    let cap = Capture::default();
    let _g = cap.install();
    let (rec, obs) = recording();
    let profile = HttpHarness::start(vec![
        server_error(),
        Reply::json(fixtures::json_body("profile.json").unwrap()),
    ])
    .await;
    let margins = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("margins.json").unwrap(),
    )])
    .await;
    let trades = HttpHarness::start(vec![
        server_error(),
        server_error(),
        Reply::json(fixtures::json_body("trades.json").unwrap()),
    ])
    .await;
    // Three clients, one scope, interleaved on one thread.
    let (a, b, t) = (
        client(&profile.base_url(), &obs),
        client(&margins.base_url(), &obs),
        client(&trades.base_url(), &obs),
    );
    let (ua, ub) = (a.user(), b.user());
    let (ra, rb, rt) = tokio::join!(ua.profile(), ub.margins(), async {
        t.clone().orders().list_trades().await
    },);
    ra.unwrap();
    rb.unwrap();
    rt.unwrap();

    let spans = cap.spans();
    let ops: Vec<&SpanRec> = spans.iter().filter(|s| s.name == OP).collect();
    assert_eq!(ops.len(), 3);
    let mut per_endpoint = BTreeMap::new();
    for attempt in spans.iter().filter(|s| s.name == ATTEMPT) {
        let op = ops
            .iter()
            .find(|o| Some(o.id) == attempt.parent)
            .expect("every attempt has its operation as parent");
        assert_eq!(attempt.field("endpoint"), op.field("endpoint"));
        assert_eq!(attempt.field("operation_id"), op.field("operation_id"));
        *per_endpoint
            .entry(op.field("endpoint").unwrap().to_string())
            .or_insert(0) += 1;
    }
    assert_eq!(
        per_endpoint,
        BTreeMap::from([
            ("/trades".to_string(), 3),
            ("/user/margins".to_string(), 1),
            ("/user/profile".to_string(), 2),
        ])
    );
    let ids: std::collections::BTreeSet<_> = ops.iter().map(|o| o.field("operation_id")).collect();
    assert_eq!(ids.len(), 3, "operation ids are distinct within a scope");
    assert_eq!(rec.counter_total(Instrument::HttpRetriesTotal), 3);
}

// ---- label content and telemetry independence ---------------------------

const ORDER_ID: &str = "151220000000000";
const SECRET: &str = "test_api_secret";
const REQUEST_TOKEN: &str = "request_token_0123";
const CHECKSUM: &str = "a8d35625a57e864a7f4dea0c182514394272ed642a506ec499e9822c6bb88ae7";

#[tokio::test]
async fn labels_and_fields_carry_no_identifier_path_body_or_secret() {
    let cap = Capture::default();
    let _g = cap.install();
    let (rec, obs) = recording();
    let h = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("order_response.json").unwrap()),
        Reply::json(fixtures::json_body("order_info.json").unwrap()),
        Reply::Respond {
            status: 403,
            content_type: "application/json",
            body: br#"{"status":"error","message":"Incorrect `api_key` or `access_token`.","error_type":"TokenException"}"#.to_vec(),
        },
        Reply::json(fixtures::json_body("generate_session.json").unwrap()),
    ])
    .await;
    let c = client(&h.base_url(), &obs);
    c.clone()
        .orders()
        .cancel_order(OrderVariety::Regular, &OrderId::new(ORDER_ID).unwrap())
        .await
        .unwrap();
    c.clone()
        .orders()
        .get_order_history(&OrderId::new(ORDER_ID).unwrap())
        .await
        .unwrap();
    c.user().profile().await.unwrap_err();
    c.session(ApiKey::new("test_api_key").unwrap())
        .exchange(
            &RequestToken::new(REQUEST_TOKEN).unwrap(),
            &ApiSecret::new(SECRET).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rec.counter(Instrument::AuthRejectionsTotal, &["http"]), 1);
    assert_eq!(
        rec.counter(
            Instrument::HttpOperationsTotal,
            &["POST", "/session/token", "sess", "ok"]
        ),
        1
    );

    let forbidden = [
        ORDER_ID,
        SECRET,
        REQUEST_TOKEN,
        CHECKSUM,
        "test_access_token",
        "test_api_key",
        "Incorrect",
        "regular",
        "127.0.0.1",
        "http://",
    ];
    let labels: Vec<String> = rec
        .dump()
        .into_iter()
        .flat_map(|(l, _)| l.values().iter().map(|v| v.to_string()).collect::<Vec<_>>())
        .collect();
    let fields: Vec<String> = cap
        .spans()
        .into_iter()
        .flat_map(|s| s.fields.into_values())
        .collect();
    let diagnostics = format!("{:?}", c.diagnostics());
    for f in forbidden {
        for v in labels.iter().chain(&fields) {
            assert!(!v.contains(f), "{f} in {v}");
        }
        if f != "Incorrect" {
            // The broker message is kept, bounded and sanitized, in the
            // failure history only.
            assert!(!diagnostics.contains(f), "{f} in diagnostics");
        }
    }
    // Every series stays within its catalogue bound.
    for i in Instrument::ALL {
        assert!(rec.series_count(*i) <= manja::kite::obs::schema::series_bound(*i));
    }
    let last = c.diagnostics().last_failures.last().cloned().unwrap();
    assert_eq!(last.http_status, Some(403));
    assert_eq!(
        last.stage,
        manja::kite::error::TransportStage::ResponseReceived
    );
}

// A scripted run whose protocol-visible results are compared across
// observability configurations.
async fn scripted(obs: Observability) -> (Vec<String>, usize) {
    let h = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("profile.json").unwrap()),
        server_error(),
        Reply::json(fixtures::json_body("orders.json").unwrap()),
        server_error(),
        Reply::json(fixtures::json_body("generate_session.json").unwrap()),
    ])
    .await;
    let c = client(&h.base_url(), &obs);
    let mut out = vec![
        format!("{:?}", c.user().profile().await.map(|r| r.data.is_some())),
        format!(
            "{:?}",
            c.clone()
                .orders()
                .list_orders()
                .await
                .map(|r| r.data.map(|d| d.len()))
        ),
        format!(
            "{:?}",
            c.clone()
                .orders()
                .cancel_order(OrderVariety::Regular, &OrderId::new(ORDER_ID).unwrap())
                .await
                .map_err(|e| {
                    let e = e.as_http().unwrap();
                    (e.kind(), e.http_status(), e.stage(), e.attempt())
                })
                .map(|_| ())
        ),
        format!(
            "{:?}",
            c.session(ApiKey::new("test_api_key").unwrap())
                .exchange(
                    &RequestToken::new(REQUEST_TOKEN).unwrap(),
                    &ApiSecret::new(SECRET).unwrap(),
                )
                .await
                .map(|r| r.data.map(|s| s.user_id))
        ),
    ];
    out.push(format!(
        "{:?}",
        Market::new(&c)
            .get_quotes::<LTPQuote>(&[])
            .await
            .map(|_| ())
            .map_err(|e| e.as_http().unwrap().kind())
    ));
    (out, h.requests().len())
}

#[tokio::test]
async fn a_stalled_adapter_changes_no_result_or_attempt_count() {
    let baseline = scripted(Observability::disabled()).await;
    let bridge = Arc::new(BridgeRecorder::new(1).unwrap());
    let stalled = scripted(Observability::with_recorder(bridge.clone())).await;
    assert!(bridge.dropped() > 0, "the adapter was saturated");
    assert_eq!(baseline, stalled);
    let (rec, obs) = recording();
    assert_eq!(baseline, scripted(obs).await);
    assert!(rec.counter_total(Instrument::HttpOperationsTotal) >= 5);
}

#[test]
fn clients_default_to_a_disabled_scope_and_share_it_with_clones() {
    let c = HTTPClient::new(Config::default()).unwrap();
    assert!(!c.observability().is_recording());
    assert!(c.observability().same_scope(c.clone().observability()));
    let d = c.with_credentials(Credentials::new("k", "t").unwrap());
    assert!(c.observability().same_scope(d.observability()));
    let (_rec, obs) = recording();
    let e = HTTPClient::with_observability(Config::default(), obs.clone()).unwrap();
    assert!(e.observability().same_scope(&obs));
}
