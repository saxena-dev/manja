//! Observability conformance (plan task S29): redaction across every sink,
//! adversarial cardinality, adapter failure, notification lag, aggregation
//! and teardown, and the absence of per-tick tasks and spans.
//!
//! Per-subsystem cases live in `http_observability`, `ticker_status`,
//! `decoder_adapter` and `obs_schema`; the conformance record maps every
//! §12.4 scenario to its test. Nothing global is installed: spans go to a
//! thread-default capture layer.

#[path = "../support/mod.rs"]
mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::{
    AccessToken, ApiKey, ApiSecret, Credentials, RequestToken,
};
use manja::kite::connect::models::{LTPQuote, OrderVariety};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::decoder::adapter::Adapter;
use manja::kite::envelope::{
    MonotonicElapsed, PayloadKind, RawObservation, ReceiveTime, SourceIdentity, SourceSequencer,
};
use manja::kite::obs::handle::Labels;
use manja::kite::obs::schema::{series_bound, SourceMode};
use manja::kite::obs::{
    BridgeRecorder, InMemoryRecorder, Instrument, MetricRecorder, Observability,
};
use manja::kite::protocol::InstrumentToken;
use manja::kite::ticker::actor::lifecycle::ReconnectLimits;
use manja::kite::ticker::actor::owner::{TickerBuilder, TickerEvent, TickerEvents, TickerLimits};
use manja::kite::ticker::Mode;
use tokio_tungstenite::tungstenite::Message;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

use support::fixtures;
use support::http::{HttpHarness, Reply};
use support::ws::{Handshake, Step, WsConnection, WsHarness};

// ---- sinks --------------------------------------------------------------

// (span id, name, fields)
type SpanRecord = (u64, &'static str, BTreeMap<String, String>);

#[derive(Clone, Default)]
struct Spans(Arc<Mutex<Vec<SpanRecord>>>);

struct Fields<'a>(&'a mut BTreeMap<String, String>);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
        self.0.insert(f.name().into(), format!("{v:?}"));
    }
    fn record_str(&mut self, f: &Field, v: &str) {
        self.0.insert(f.name().into(), v.into());
    }
}

impl<S: Subscriber> Layer<S> for Spans {
    fn on_new_span(&self, a: &Attributes<'_>, id: &Id, _: Context<'_, S>) {
        let mut m = BTreeMap::new();
        a.record(&mut Fields(&mut m));
        self.0
            .lock()
            .unwrap()
            .push((id.into_u64(), a.metadata().name(), m));
    }
    fn on_record(&self, id: &Id, r: &Record<'_>, _: Context<'_, S>) {
        let mut spans = self.0.lock().unwrap();
        if let Some(s) = spans.iter_mut().rev().find(|s| s.0 == id.into_u64()) {
            r.record(&mut Fields(&mut s.2));
        }
    }
}

impl Spans {
    fn install(&self) -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(tracing_subscriber::registry().with(self.clone()))
    }
    fn text(&self) -> String {
        format!("{:?}", self.0.lock().unwrap())
    }
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

// One recorder feeding both an in-memory store and a bounded bridge.
struct Both(Arc<InMemoryRecorder>, Arc<BridgeRecorder>);

impl MetricRecorder for Both {
    fn counter_add(&self, l: &Labels, v: u64) {
        self.0.counter_add(l, v);
        self.1.counter_add(l, v);
    }
    fn gauge_set(&self, l: &Labels, v: f64) {
        self.0.gauge_set(l, v);
        self.1.gauge_set(l, v);
    }
    fn histogram_record(&self, l: &Labels, s: f64) {
        self.0.histogram_record(l, s);
        self.1.histogram_record(l, s);
    }
}

fn http_config(base: &str) -> Config {
    Config::new(base).with_limits(
        HttpLimits::default()
            .with_scheduler(SchedulerLimits::default().with_read_attempts(1).unwrap()),
    )
}

// ---- R-02: redaction sweep over spans, labels, snapshots and adapter
// buffers -------------------------------------------------------------------

// Credential-shaped sentinels (24 or more letters and digits) and short
// correlation inputs.
const API_KEY: &str = "SENTINELapiKey0000000000001";
const ACCESS: &str = "SENTINELaccessToken00000000002";
const REQUEST: &str = "SENTINELrequestToken0000000003";
const SECRET: &str = "SENTINELapiSecret000000000004";
const ORDER_ID: &str = "SENTINELorder5";
const SYMBOL: &str = "SENTINELSYM6";
const SESSION_TOKEN: &str = "SENTINELsessionAccess000000007";

#[tokio::test]
async fn no_seeded_value_reaches_any_telemetry_sink() {
    let spans = Spans::default();
    let _g = spans.install();
    let memory = Arc::new(InMemoryRecorder::new());
    let bridge = Arc::new(BridgeRecorder::new(BridgeRecorder::CAPACITY_RANGE.1).unwrap());
    let obs = Observability::with_recorder(Arc::new(Both(memory.clone(), bridge.clone())));

    // HTTP: a success, a broker error echoing the token, a cancellation by
    // an order ID, quote keys in the query, an exchange whose response
    // carries new tokens, and an invalidation carrying a token in its URL.
    let session = fixtures::json_body("generate_session.json")
        .unwrap()
        .replace(
            "\"access_token\": \"XXXXXX\"",
            &format!("\"access_token\": \"{SESSION_TOKEN}\""),
        );
    let echo = format!(
        r#"{{"status":"error","message":"Incorrect access_token={ACCESS} for {ORDER_ID}","error_type":"TokenException"}}"#
    );
    let h = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("profile.json").unwrap()),
        Reply::Respond {
            status: 403,
            content_type: "application/json",
            body: echo.into_bytes(),
        },
        Reply::json(fixtures::json_body("order_response.json").unwrap()),
        Reply::json(fixtures::json_body("ltp.json").unwrap()),
        Reply::json(session),
        Reply::json(fixtures::json_body("session_logout.json").unwrap()),
    ])
    .await;
    let c = HTTPClient::with_observability(http_config(&h.base_url()), obs.clone())
        .unwrap()
        .with_credentials(Credentials::new(API_KEY, ACCESS).unwrap());
    c.user().profile().await.unwrap();
    let err = c.user().margins().await.unwrap_err();
    c.orders()
        .cancel_order(OrderVariety::Regular, ORDER_ID)
        .await
        .unwrap();
    let symbol = format!("NSE:{SYMBOL}");
    c.market()
        .get_quotes::<LTPQuote>(&[symbol.as_str()])
        .await
        .unwrap();
    let s = c
        .session(ApiKey::new(API_KEY).unwrap())
        .exchange(
            &RequestToken::new(REQUEST).unwrap(),
            &ApiSecret::new(SECRET).unwrap(),
        )
        .await
        .unwrap()
        .data
        .unwrap();
    c.session(ApiKey::new(API_KEY).unwrap())
        .invalidate(&AccessToken::new(ACCESS).unwrap())
        .await
        .unwrap();

    // Ticker: the credentials travel in the handshake URL; a broker text
    // echoes the token.
    let w = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: vec![
            Step::Send(Message::Text(format!(
                r#"{{"type":"error","data":"bad {ACCESS}"}}"#
            ))),
            Step::Send(Message::Binary(vec![0])),
        ],
    }])
    .await;
    let (handle, mut events, guard) =
        TickerBuilder::new(Credentials::new(API_KEY, ACCESS).unwrap())
            .url(w.url())
            .observability(obs.clone())
            .spawn()
            .unwrap();
    handle
        .subscribe([InstrumentToken::new(408065)], Mode::Full)
        .await
        .unwrap();
    let mut raw = 0;
    while raw < 2 {
        if let Some(Ok(TickerEvent::Raw(_))) = events.next().await {
            raw += 1;
        }
    }
    let status = format!("{:?}", handle.status());
    handle.shutdown().await.unwrap();
    while events.next().await.is_some() {}
    guard.join().await;

    // Decoder: a malformed text payload containing the token.
    let mut seq = SourceSequencer::new(SourceIdentity::generate());
    seq.begin_epoch();
    let o = RawObservation::new(
        seq.next_key(),
        PayloadKind::Text,
        ReceiveTime::from_unix_nanos(0),
        MonotonicElapsed::from_nanos(0),
        format!("{{\"type\":\"order\",\"data\":\"{ACCESS}\""),
        1 << 20,
    )
    .unwrap();
    let decoded = Adapter::new(SourceMode::Live, &obs)
        .with_spans(true)
        .decode(&o)
        .unwrap();

    let buffered = bridge.drain();
    assert!(!buffered.is_empty(), "the adapter buffer was exercised");
    let sinks = [
        ("spans", spans.text()),
        ("labels", format!("{:?}", memory.dump())),
        ("adapter buffer", format!("{buffered:?}")),
        ("http diagnostics", format!("{:?}", c.diagnostics())),
        ("ticker status", status),
        ("decode diagnostics", format!("{:?}", decoded.diagnostics)),
        ("errors", format!("{err} {err:?}")),
        ("session debug", format!("{s:?}")),
    ];
    // The exchange checksum, SHA-256(api_key + request_token + api_secret).
    let checksum: String = {
        use sha2::{Digest, Sha256};
        Sha256::digest(format!("{API_KEY}{REQUEST}{SECRET}"))
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    for (sink, text) in &sinks {
        for seeded in [API_KEY, ACCESS, REQUEST, SECRET, SESSION_TOKEN] {
            assert!(!text.contains(seeded), "{seeded} reached {sink}: {text}");
        }
        assert!(!text.contains(&checksum), "checksum reached {sink}");
        // Correlation inputs are never labels or span fields.
        if matches!(*sink, "spans" | "labels" | "adapter buffer") {
            for id in [ORDER_ID, SYMBOL] {
                assert!(!text.contains(id), "{id} reached {sink}");
            }
            assert!(!text.contains("127.0.0.1"), "no URL in {sink}");
        }
    }
    // The echoed token was present and masked, not merely absent.
    assert!(format!("{:?}", c.diagnostics()).contains("<redacted>"));
}

// ---- R-04: adversarial cardinality ------------------------------------

#[tokio::test]
async fn a_hundred_thousand_dynamic_identifiers_stay_within_series_bounds() {
    let rec = Arc::new(InMemoryRecorder::new());
    let obs = Observability::with_recorder(rec.clone());
    // HTTP: every request is refused before dispatch, so no server is
    // involved; each carries a distinct order ID or quote key.
    let c = HTTPClient::with_observability(http_config("http://127.0.0.1:1"), obs.clone())
        .unwrap()
        .with_credentials(Credentials::new("k", "t").unwrap());
    for i in 0..50_000u32 {
        let id = format!("bad-{i}");
        let _ = c.orders().cancel_order(OrderVariety::Regular, &id).await;
        let key = format!("X{i}");
        let _ = c.market().get_quotes::<LTPQuote>(&[key.as_str()]).await;
    }
    // Decoder: 100 000 distinct source keys.
    let adapter = Adapter::new(SourceMode::Standalone, &obs);
    let mut seq = SourceSequencer::new(SourceIdentity::generate());
    seq.begin_epoch();
    for i in 0..100_000u32 {
        let o = RawObservation::new(
            seq.next_key(),
            PayloadKind::Binary,
            ReceiveTime::from_unix_nanos(i as i64),
            MonotonicElapsed::from_nanos(i as u64),
            i.to_be_bytes().to_vec(),
            1 << 20,
        )
        .unwrap();
        adapter.decode(&o).unwrap();
    }
    assert_eq!(rec.counter_total(Instrument::HttpOperationsTotal), 100_000);
    let mut total = 0;
    for i in Instrument::ALL {
        let n = rec.series_count(*i);
        assert!(n <= series_bound(*i), "{i:?}: {n}");
        total += n;
    }
    assert!(
        total < 20,
        "a handful of series, not one per identifier: {total}"
    );
    assert_eq!(rec.dump().len(), total);
}

// ---- R-03: supported-adapter failure ------------------------------------

fn flood(n: usize) -> Vec<Step> {
    (0..n)
        .map(|i| Step::Send(Message::Binary((i as u32).to_be_bytes().to_vec())))
        .collect()
}

async fn scripted_ticker(obs: Observability) -> Vec<String> {
    let w = WsHarness::start(vec![
        WsConnection {
            handshake: Handshake::Accept,
            steps: {
                let mut s = flood(50);
                s.push(Step::Eof);
                s
            },
        },
        WsConnection {
            handshake: Handshake::Accept,
            steps: flood(10),
        },
    ])
    .await;
    let limits = TickerLimits::default().with_reconnect(
        ReconnectLimits::default()
            .with_backoff(Duration::from_millis(50), Duration::from_millis(50))
            .unwrap()
            .with_jitter_seed(3),
    );
    let (handle, mut events, guard) = TickerBuilder::new(Credentials::new("k", "t").unwrap())
        .url(w.url())
        .limits(limits)
        .observability(obs)
        .spawn()
        .unwrap();
    handle
        .subscribe([InstrumentToken::new(1)], Mode::LTP)
        .await
        .unwrap();
    let mut out = Vec::new();
    let mut raw = 0;
    while raw < 60 {
        match tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .unwrap()
        {
            Some(Ok(TickerEvent::Raw(r))) => {
                raw += 1;
                out.push(format!(
                    "raw {} {} {:?}",
                    r.source().connection_epoch().0,
                    r.source().ingress_sequence(),
                    r.payload().as_bytes()
                ));
            }
            Some(Ok(TickerEvent::Lifecycle(l))) => out.push(format!(
                "{} {} {}",
                l.source().connection_epoch().0,
                l.source().ingress_sequence(),
                match l.kind() {
                    manja::kite::envelope::LifecycleKind::Backoff { .. } => "Backoff".to_string(),
                    manja::kite::envelope::LifecycleKind::Gap(_) => "Gap".to_string(),
                    k => format!("{k:?}"),
                }
            )),
            other => panic!("{other:?}"),
        }
    }
    handle.shutdown().await.unwrap();
    let rest = drain(&mut events).await;
    out.extend(rest);
    out.push(format!("{:?}", guard.join().await));
    out
}

async fn drain(events: &mut TickerEvents) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(item) = events.next().await {
        out.push(format!(
            "{:?}",
            item.map(|e| matches!(e, TickerEvent::Raw(_)))
        ));
    }
    out
}

#[tokio::test]
async fn stalled_or_never_drained_adapters_change_no_protocol_outcome() {
    let baseline = scripted_ticker(Observability::disabled()).await;
    // Stalled: a bridge of capacity 1 that is never drained.
    let stalled = Arc::new(BridgeRecorder::new(1).unwrap());
    let a = scripted_ticker(Observability::with_recorder(stalled.clone())).await;
    // Disconnected: an exporter that has gone away, so the bridge fills
    // and stays full.
    let gone = Arc::new(BridgeRecorder::new(16).unwrap());
    let b = scripted_ticker(Observability::with_recorder(gone.clone())).await;
    assert_eq!(
        a, baseline,
        "ordering, reconnect and terminal outcome unchanged"
    );
    assert_eq!(b, baseline);
    // Local drop accounting is readable; the buffer stays bounded.
    assert!(stalled.dropped() > 0);
    assert!(stalled.len() <= 1);
    assert!(gone.dropped() > 0);
    assert!(gone.len() <= 16);
    // No retained leak: once the ticker is gone, only this test holds the
    // recorders.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(Arc::strong_count(&stalled), 1);
    assert_eq!(Arc::strong_count(&gone), 1);
}

// ---- notification lag, aggregation and teardown -------------------------

#[tokio::test]
async fn a_slow_status_reader_sees_coalesced_changes_and_delays_nothing() {
    let w = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: flood(20),
    }])
    .await;
    let (handle, mut events, _guard) = TickerBuilder::new(Credentials::new("k", "t").unwrap())
        .url(w.url())
        .spawn()
        .unwrap();
    let before = handle.status().snapshot_revision;
    // Many state changes while nobody reads status.
    for t in 0..50u32 {
        handle
            .subscribe([InstrumentToken::new(t + 1)], Mode::LTP)
            .await
            .unwrap();
    }
    let seen = handle.changed(before).await;
    let lag = seen.snapshot_revision - before;
    assert!(lag > 1, "coalesced: {lag} changes in one notification");
    // The primary stream is unaffected: every observation, in order.
    let mut seqs = Vec::new();
    while seqs.len() < 20 {
        if let Some(Ok(TickerEvent::Raw(r))) = events.next().await {
            seqs.push(r.source().ingress_sequence());
        }
    }
    assert!(seqs.windows(2).all(|w| w[1] > w[0]));
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn gauges_aggregate_across_clones_and_owners_and_unregister_on_teardown() {
    let rec = Arc::new(InMemoryRecorder::new());
    let obs = Observability::with_recorder(rec.clone());
    let w1 = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: flood(7),
    }])
    .await;
    let w2 = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: flood(11),
    }])
    .await;
    let spawn = |url: String| {
        TickerBuilder::new(Credentials::new("k", "t").unwrap())
            .url(url)
            .observability(obs.clone())
            .spawn()
            .unwrap()
    };
    let (h1, e1, g1) = spawn(w1.url());
    let (h2, e2, g2) = spawn(w2.url());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (s1, s2) = (h1.status(), h2.status());
    let queued = rec
        .gauge(Instrument::SdkQueueMessages, &["raw_primary"])
        .unwrap();
    assert_eq!(
        queued as usize,
        s1.queue.messages + s2.queue.messages,
        "summed"
    );
    assert_eq!(
        rec.gauge(Instrument::TickerConnectionsActive, &[]),
        Some(2.0)
    );
    let oldest = obs
        .collect_gauges()
        .into_iter()
        .find(|g| {
            Labels::instrument(&g.labels) == Instrument::SdkQueueOldestAge
                && g.labels.values() == ["raw_primary"]
        })
        .unwrap()
        .value;
    let max = s1
        .queue
        .oldest_age
        .max(s2.queue.oldest_age)
        .unwrap()
        .as_secs_f64();
    assert!(
        oldest >= max * 0.9,
        "the maximum age across owners: {oldest} vs {max}"
    );
    for (h, mut e, g) in [(h1, e1, g1), (h2, e2, g2)] {
        h.shutdown().await.unwrap();
        while e.next().await.is_some() {}
        g.join().await;
    }
    assert!(
        obs.collect_gauges().iter().all(|g| g.value == 0.0),
        "no ghost gauges"
    );
}

// ---- R-06: no task or span per tick -------------------------------------

#[tokio::test]
async fn streaming_creates_no_task_or_span_per_message() {
    let spans = Spans::default();
    let _g = spans.install();
    let metrics = tokio::runtime::Handle::current().metrics();
    let w = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: flood(2000),
    }])
    .await;
    let (handle, mut events, _guard) = TickerBuilder::new(Credentials::new("k", "t").unwrap())
        .url(w.url())
        .spawn()
        .unwrap();
    let mut raw = 0;
    let mut tasks = Vec::new();
    let mut span_counts = Vec::new();
    while raw < 2000 {
        if let Some(Ok(TickerEvent::Raw(_))) = events.next().await {
            raw += 1;
            if raw % 500 == 0 {
                tasks.push(metrics.num_alive_tasks());
                span_counts.push(spans.count());
            }
        }
    }
    assert!(
        tasks.windows(2).all(|w| w[0] == w[1]),
        "tasks constant: {tasks:?}"
    );
    assert!(
        span_counts.windows(2).all(|w| w[0] == w[1]),
        "spans constant: {span_counts:?}"
    );
    handle.shutdown().await.unwrap();
}
