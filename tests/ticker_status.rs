//! Ticker instrumentation and queryable status (plan task S20), against the
//! loopback WebSocket harness, with the SDK's in-memory recorder and a
//! test-local `tracing` capture layer. Nothing global is installed.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use manja::kite::connect::credentials::Credentials;
use manja::kite::envelope::{RunId, SourceIdentity};
use manja::kite::obs::{BridgeRecorder, InMemoryRecorder, Instrument, Observability};
use manja::kite::protocol::InstrumentToken;
use manja::kite::ticker::actor::lifecycle::ReconnectLimits;
use manja::kite::ticker::actor::owner::{
    TerminalReason, TickerBuilder, TickerError, TickerEvent, TickerEvents, TickerLimits,
    TickerState,
};
use manja::kite::ticker::Mode;
use tokio_tungstenite::tungstenite::Message;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Instrument as _, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

use support::capture::read_ticker_fixture;
use support::ws::{Handshake, Step, WsConnection, WsHarness};

// ---- span capture -------------------------------------------------------

#[derive(Clone, Debug)]
struct SpanRec {
    id: u64,
    name: &'static str,
    parent: Option<u64>,
    fields: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<SpanRec>>>);

struct Fields<'a>(&'a mut BTreeMap<String, String>);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().into(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
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
        if let Some(s) = spans.iter_mut().rev().find(|s| s.id == id.into_u64()) {
            values.record(&mut Fields(&mut s.fields));
        }
    }
}

impl Capture {
    fn install(&self) -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(tracing_subscriber::registry().with(self.clone()))
    }

    fn named(&self, name: &str) -> Vec<SpanRec> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.name == name)
            .cloned()
            .collect()
    }
}

// ---- helpers ------------------------------------------------------------

const FEED: &str = "feedSENTINEL01";
const TOKEN: &str = "SENTINELtoken0002";

fn identity() -> SourceIdentity {
    SourceIdentity::new("manja", RunId::from_u128(7), FEED).unwrap()
}

fn builder(url: &str, obs: &Observability, limits: TickerLimits) -> TickerBuilder {
    TickerBuilder::new(Credentials::new("test_api_key", TOKEN).unwrap())
        .url(url)
        .identity(identity())
        .observability(obs.clone())
        .limits(limits)
}

fn quick() -> TickerLimits {
    TickerLimits::default().with_reconnect(
        ReconnectLimits::default()
            .with_backoff(Duration::from_millis(50), Duration::from_millis(50))
            .unwrap()
            .with_jitter_seed(1),
    )
}

fn recording() -> (Arc<InMemoryRecorder>, Observability) {
    let rec = Arc::new(InMemoryRecorder::new());
    let obs = Observability::with_recorder(rec.clone());
    (rec, obs)
}

fn accept(steps: Vec<Step>) -> WsConnection {
    WsConnection {
        handshake: Handshake::Accept,
        steps,
    }
}

fn describe(item: &Result<TickerEvent, TickerError>) -> String {
    match item {
        Ok(TickerEvent::Raw(r)) => format!("raw:{}", r.payload().len()),
        Ok(TickerEvent::Lifecycle(l)) => match l.kind() {
            manja::kite::envelope::LifecycleKind::Backoff { .. } => "Backoff".into(),
            manja::kite::envelope::LifecycleKind::Gap(_) => "Gap".into(),
            other => format!("{other:?}"),
        },
        Ok(_) => "other".into(),
        Err(e) => format!("err:{:?}", e.reason()),
    }
}

async fn until(events: &mut TickerEvents, stop: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), events.next()).await {
            Ok(Some(item)) => {
                let d = describe(&item);
                let done = stop(&d);
                out.push(d);
                if done {
                    return out;
                }
            }
            Ok(None) => return out,
            Err(_) => panic!("stalled after {out:?}"),
        }
    }
}

fn gauges_settled(obs: &Observability) -> bool {
    obs.collect_gauges().iter().all(|g| g.value == 0.0)
}

// ---- R-02 ---------------------------------------------------------------

async fn raw_count(events: &mut TickerEvents, n: usize) {
    let mut seen = 0;
    while seen < n {
        match tokio::time::timeout(Duration::from_secs(5), events.next()).await {
            Ok(Some(Ok(TickerEvent::Raw(_)))) => seen += 1,
            Ok(Some(_)) => {}
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn complete_messages_are_counted_heartbeats_included_never_packets() {
    let (rec, obs) = recording();
    let multi = read_ticker_fixture("protocol/multi_packet.bin");
    let mut steps: Vec<Step> = (0..5)
        .map(|_| Step::Send(Message::Binary(vec![0])))
        .collect();
    steps.push(Step::Send(Message::Binary(multi.clone())));
    steps.push(Step::Send(Message::Text("{\"type\":\"message\"}".into())));
    steps.push(Step::Send(Message::Ping(vec![9])));
    let h = WsHarness::start(vec![accept(steps)]).await;
    let (handle, mut events, _guard) = builder(&h.url(), &obs, quick()).spawn().unwrap();
    raw_count(&mut events, 7).await;
    assert_eq!(
        rec.counter(Instrument::TickerReceivedMessagesTotal, &["binary"]),
        6,
        "five heartbeats and one multi-packet message"
    );
    assert_eq!(
        rec.counter(Instrument::TickerReceivedBytesTotal, &["binary"]),
        5 + multi.len() as u64
    );
    assert_eq!(
        rec.counter(Instrument::TickerReceivedMessagesTotal, &["text"]),
        1
    );
    let status = handle.status();
    assert!(status.last_heartbeat_age.is_some());
    assert!(status.last_message_age.unwrap() <= status.last_heartbeat_age.unwrap());
    handle.shutdown().await.unwrap();
}

// ---- R-01 ---------------------------------------------------------------

#[tokio::test]
async fn handshakes_sockets_reconnects_restores_and_commands_have_their_own_facts() {
    let cap = Capture::default();
    let _g = cap.install();
    let (rec, obs) = recording();
    let h = WsHarness::start(vec![
        accept(vec![Step::Wait(Duration::from_millis(100)), Step::Eof]),
        accept(vec![]),
    ])
    .await;
    let (handle, mut events, guard) = builder(&h.url(), &obs, quick()).spawn().unwrap();
    handle
        .subscribe([InstrumentToken::new(1)], Mode::LTP)
        .await
        .unwrap();
    until(&mut events, |d| d.starts_with("Active")).await;
    let active = |rec: &InMemoryRecorder| {
        rec.gauge(Instrument::TickerConnectionsActive, &[])
            .unwrap_or(0.0)
    };
    assert_eq!(active(&rec), 1.0);
    // The first connection ends; the reconnect starts, then restores.
    until(&mut events, |d| d.starts_with("Disconnected")).await;
    until(&mut events, |d| d.starts_with("Active")).await;
    assert_eq!(active(&rec), 1.0, "one socket at a time");
    assert_eq!(
        rec.counter(Instrument::TickerConnectionAttemptsTotal, &["ok"]),
        2
    );
    assert_eq!(
        rec.histogram(Instrument::TickerConnectDuration, &["ok"]).0,
        2
    );
    assert_eq!(rec.counter(Instrument::TickerReconnectsTotal, &["eof"]), 1);
    assert_eq!(rec.counter_total(Instrument::TickerReconnectsTotal), 1);
    assert_eq!(
        rec.histogram(Instrument::TickerRestoreDuration, &["sent"])
            .0,
        2
    );
    // Command decisions.
    handle
        .set_mode([InstrumentToken::new(5)], Mode::Full)
        .await
        .unwrap_err();
    assert_eq!(
        rec.counter(Instrument::TickerCommandsTotal, &["subscribe", "accepted"]),
        1
    );
    assert_eq!(
        rec.counter(Instrument::TickerCommandsTotal, &["set_mode", "rejected"]),
        1
    );
    // Spans: one connection span per attempt, with the feed and epoch as
    // fields.
    let connections = cap.named("manja.ticker.connection");
    assert_eq!(connections.len(), 2);
    assert_eq!(connections[0].fields.get("reason").unwrap(), "start");
    assert_eq!(connections[1].fields.get("reason").unwrap(), "eof");
    assert_eq!(connections[1].fields.get("connection_epoch").unwrap(), "2");
    assert!(connections
        .iter()
        .all(|c| c.fields.get("feed_id").unwrap() == FEED));
    assert!(connections
        .iter()
        .all(|c| c.fields.get("result").unwrap() == "ok"));
    let restores = cap.named("manja.ticker.restore");
    assert_eq!(restores.len(), 2);
    assert_eq!(restores[1].fields.get("instrument_count").unwrap(), "1");
    assert_eq!(restores[1].fields.get("sent_count").unwrap(), "2");
    let commands = cap.named("manja.ticker.command");
    assert_eq!(commands[1].fields.get("decision").unwrap(), "rejected");
    assert_eq!(commands[1].fields.get("rejection").unwrap(), "not_desired");
    // A handshake rejection counts as its own result.
    handle.shutdown().await.unwrap();
    guard.join().await;
    assert_eq!(active(&rec), 0.0);
    assert_eq!(
        rec.histogram(Instrument::TickerShutdownDuration, &["clean"])
            .0,
        1
    );
    assert_eq!(cap.named("manja.ticker.shutdown").len(), 1);
    let h = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Reject(403),
        steps: vec![],
    }])
    .await;
    let (_handle, mut events, _guard) = builder(&h.url(), &obs, quick()).spawn().unwrap();
    until(&mut events, |d| d.starts_with("err:")).await;
    assert_eq!(
        rec.counter(
            Instrument::TickerConnectionAttemptsTotal,
            &["auth_rejected"]
        ),
        1
    );
}

#[tokio::test]
async fn caller_context_survives_the_mailbox_without_mixing_callers() {
    let cap = Capture::default();
    let _g = cap.install();
    let (_rec, obs) = recording();
    let h = WsHarness::start(vec![accept(vec![])]).await;
    let (handle, mut events, _guard) = builder(&h.url(), &obs, quick()).spawn().unwrap();
    until(&mut events, |d| d.starts_with("Active")).await;
    let caller = |name: &'static str, token: u32| {
        let handle = handle.clone();
        let span = tracing::debug_span!("caller", name);
        async move {
            for _ in 0..5 {
                handle
                    .subscribe([InstrumentToken::new(token)], Mode::Quote)
                    .await
                    .unwrap();
                handle
                    .unsubscribe([InstrumentToken::new(token)])
                    .await
                    .unwrap();
                tokio::task::yield_now().await;
            }
        }
        .instrument(span)
    };
    tokio::join!(caller("a", 1), caller("b", 2));
    let callers: BTreeMap<u64, String> = cap
        .named("caller")
        .into_iter()
        .map(|s| (s.id, s.fields.get("name").unwrap().clone()))
        .collect();
    let commands = cap.named("manja.ticker.command");
    assert_eq!(commands.len(), 20);
    let mut per_caller = BTreeMap::new();
    for c in &commands {
        let parent = callers
            .get(&c.parent.expect("a command span has its caller as parent"))
            .expect("parent is a caller span");
        assert_eq!(c.fields.get("decision").unwrap(), "accepted");
        *per_caller.entry(parent.clone()).or_insert(0) += 1;
    }
    assert_eq!(
        per_caller,
        BTreeMap::from([("a".into(), 10), ("b".into(), 10)])
    );
    handle.shutdown().await.unwrap();
}

// ---- R-03 / R-04 --------------------------------------------------------

#[tokio::test]
async fn status_ages_advance_at_inspection_and_teardown_leaves_no_ghost_gauge() {
    let (rec, obs) = recording();
    let h = WsHarness::start(vec![accept(
        (0..20)
            .map(|_| Step::Send(Message::Binary(vec![0; 8])))
            .collect(),
    )])
    .await;
    let (handle, mut events, guard) = builder(&h.url(), &obs, quick()).spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Nothing read yet: 3 lifecycle events and 20 observations wait.
    let first = handle.status();
    assert_eq!(first.state, TickerState::Active);
    assert_eq!(first.queue.messages, 23);
    assert!(first.queue.retained_bytes >= 20 * 8);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = handle.status();
    assert_eq!(
        second.snapshot_revision, first.snapshot_revision,
        "no state change"
    );
    assert!(second.last_message_age.unwrap() > first.last_message_age.unwrap());
    assert!(second.queue.oldest_age.unwrap() > first.queue.oldest_age.unwrap());
    assert!(second.taken_at > first.taken_at);
    assert_eq!(
        second.last_heartbeat_age, None,
        "8-byte messages are not heartbeats"
    );
    // Queue gauges sum to the same measures, and the oldest age is computed
    // at collection.
    assert_eq!(
        rec.gauge(Instrument::SdkQueueMessages, &["raw_primary"]),
        Some(23.0)
    );
    let age = obs
        .collect_gauges()
        .into_iter()
        .find(|g| {
            manja::kite::obs::handle::Labels::instrument(&g.labels) == Instrument::SdkQueueOldestAge
                && g.labels.values() == ["raw_primary"]
        })
        .unwrap()
        .value;
    assert!(age >= 0.3, "{age}");
    raw_count(&mut events, 20).await;
    assert_eq!(handle.status().queue.messages, 0);
    handle.shutdown().await.unwrap();
    while events.next().await.is_some() {}
    guard.join().await;
    drop((handle, events));
    assert!(gauges_settled(&obs), "{:?}", obs.collect_gauges());
}

#[tokio::test]
async fn failures_are_kept_in_bounded_history_and_changes_are_notified() {
    let (_rec, obs) = recording();
    let h = WsHarness::start(vec![
        accept(vec![Step::Eof]),
        accept(vec![Step::Close]),
        accept(vec![]),
    ])
    .await;
    let (handle, mut events, _guard) = builder(&h.url(), &obs, quick()).spawn().unwrap();
    let before = handle.status().snapshot_revision;
    let changed = handle.changed(before).await;
    assert!(changed.snapshot_revision > before);
    let mut actives = 0;
    while actives < 3 {
        if until(&mut events, |d| d.starts_with("Active"))
            .await
            .last()
            .unwrap()
            .starts_with("Active")
        {
            actives += 1;
        }
    }
    let failures = handle.status().last_failures;
    let reasons: Vec<TerminalReason> = failures.iter().map(|f| f.reason.clone()).collect();
    assert_eq!(
        reasons,
        [
            TerminalReason::Disconnected(manja::kite::envelope::DisconnectReason::Eof),
            TerminalReason::Disconnected(manja::kite::envelope::DisconnectReason::RemoteClose),
        ]
    );
    assert_eq!(failures[0].connection_epoch.0, 1);
    assert_eq!(failures[1].connection_epoch.0, 2);
    handle.shutdown().await.unwrap();
}

// ---- R-05 / R-06 --------------------------------------------------------

async fn scripted(obs: Observability) -> Vec<String> {
    let h = WsHarness::start(vec![
        accept(vec![
            Step::Send(Message::Binary(vec![0])),
            Step::Wait(Duration::from_millis(50)),
            Step::Eof,
        ]),
        accept(vec![Step::Send(Message::Binary(vec![1, 2, 3]))]),
    ])
    .await;
    let (handle, mut events, _guard) = builder(&h.url(), &obs, quick()).spawn().unwrap();
    handle
        .subscribe([InstrumentToken::new(9)], Mode::Full)
        .await
        .unwrap();
    let mut out = until(&mut events, |d| d == "raw:3").await;
    handle.shutdown().await.unwrap();
    out.extend(until(&mut events, |_| false).await);
    out
}

#[tokio::test]
async fn absent_or_stalled_telemetry_changes_no_delivery() {
    let baseline = scripted(Observability::disabled()).await;
    let bridge = Arc::new(BridgeRecorder::new(1).unwrap());
    let stalled = scripted(Observability::with_recorder(bridge.clone())).await;
    assert!(bridge.dropped() > 0, "the adapter was saturated");
    assert_eq!(baseline, stalled);
    let (rec, obs) = recording();
    assert_eq!(baseline, scripted(obs).await);
    assert!(rec.counter_total(Instrument::TickerReceivedMessagesTotal) >= 2);
}

#[tokio::test]
async fn no_label_carries_feed_epoch_token_or_broker_text() {
    let (rec, obs) = recording();
    let h = WsHarness::start(vec![
        accept(vec![
            Step::Send(Message::Text(
                "{\"type\":\"error\",\"data\":\"SENTINELbroker\"}".into(),
            )),
            Step::Eof,
        ]),
        WsConnection {
            handshake: Handshake::Reject(403),
            steps: vec![],
        },
    ])
    .await;
    let (_handle, mut events, _guard) = builder(&h.url(), &obs, quick()).spawn().unwrap();
    until(&mut events, |d| d.starts_with("err:")).await;
    let dump = rec.dump();
    assert!(!dump.is_empty());
    for (labels, _) in dump {
        for v in labels.values() {
            for forbidden in [FEED, TOKEN, "SENTINELbroker", "test_api_key", "manja"] {
                assert!(!v.contains(forbidden), "{forbidden} in {v}");
            }
            assert!(v.parse::<u64>().is_err(), "numeric label {v}");
        }
    }
    for i in Instrument::ALL {
        assert!(rec.series_count(*i) <= manja::kite::obs::schema::series_bound(*i));
    }
}
