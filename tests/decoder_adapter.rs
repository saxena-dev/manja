//! Provenance adaptation and deterministic replay.
//!
//! "Live" observations are built as the ticker builds them; "captured"
//! copies are the same envelopes serialized and read back, which is enough
//! to model a captured origin without a storage format.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use manja::kite::decoder::adapter::{Adapter, DecodedEvent, VERSIONS};
use manja::kite::decoder::framing::{frame, DecodeDiagnosticKind, FramingLimits, Message};
use manja::kite::decoder::packets::{decode, Packet};
use manja::kite::envelope::{
    MonotonicElapsed, PayloadKind, RawObservation, ReceiveTime, RunId, SourceIdentity,
    SourceSequencer,
};
use manja::kite::obs::schema::SourceMode;
use manja::kite::obs::{InMemoryRecorder, Instrument, Observability};
use tracing::span::{Attributes, Id};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

use support::capture::{read_capture, read_real_capture, read_ticker_fixture};

const FEED: &str = "feedSENTINEL02";

fn sequencer() -> SourceSequencer {
    let mut s =
        SourceSequencer::new(SourceIdentity::new("manja", RunId::from_u128(3), FEED).unwrap());
    s.begin_epoch();
    s
}

fn observe(s: &mut SourceSequencer, kind: PayloadKind, bytes: Vec<u8>) -> RawObservation {
    RawObservation::new(
        s.next_key(),
        kind,
        ReceiveTime::from_unix_nanos(1_700_000_000_000_000_000),
        MonotonicElapsed::from_nanos(5),
        bytes,
        1 << 20,
    )
    .unwrap()
}

fn captured(o: &RawObservation) -> RawObservation {
    serde_json::from_str(&serde_json::to_string(o).unwrap()).unwrap()
}

fn quiet() -> Adapter {
    Adapter::new(SourceMode::Live, &Observability::disabled())
}

#[test]
fn every_event_carries_its_source_packet_index_and_versions() {
    let mut s = sequencer();
    let o = observe(
        &mut s,
        PayloadKind::Binary,
        read_ticker_fixture("protocol/multi_packet.bin"),
    );
    let d = quiet().decode(&o).unwrap();
    assert_eq!(d.versions, VERSIONS);
    assert!(d.diagnostics.is_empty());
    let indices: Vec<u16> = d
        .events
        .iter()
        .map(|e| match e {
            DecodedEvent::Packet {
                source,
                packet_index,
                ..
            } => {
                assert_eq!(source, o.source());
                *packet_index
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(indices, [0, 1]);
}

#[test]
fn failures_attach_diagnostics_and_leave_the_raw_evidence_intact() {
    let mut s = sequencer();
    for (name, kind) in [
        ("truncated", DecodeDiagnosticKind::Truncated),
        ("trailing_bytes", DecodeDiagnosticKind::TrailingBytes),
        ("unknown_size", DecodeDiagnosticKind::UnknownLength),
    ] {
        let bytes = read_ticker_fixture(&format!("protocol/{name}.bin"));
        let o = observe(&mut s, PayloadKind::Binary, bytes.clone());
        let before = o.clone();
        let d = quiet().decode(&o).unwrap();
        assert_eq!(d.diagnostics.len(), 1, "{name}");
        assert_eq!(d.diagnostics[0].kind, kind, "{name}");
        assert_eq!(&d.diagnostics[0].source, o.source());
        assert!(d.events.is_empty(), "{name}");
        assert_eq!(o, before, "{name}");
        assert_eq!(o.payload().as_bytes(), bytes, "raw bytes still readable");
    }
}

#[test]
fn live_and_captured_copies_decode_identically_on_every_capture_record() {
    let capture = read_real_capture();
    let mut s = sequencer();
    let live = Adapter::new(SourceMode::Live, &Observability::disabled());
    let (rec, obs) = {
        let rec = Arc::new(InMemoryRecorder::new());
        (rec.clone(), Observability::with_recorder(rec))
    };
    let replay = Adapter::new(SourceMode::Replay, &obs);
    let mut packets = 0;
    for r in read_capture(&capture).unwrap() {
        let o = observe(&mut s, PayloadKind::Binary, r.payload.to_vec());
        let copy = captured(&o);
        let a = live.decode(&o).unwrap();
        let b = replay.decode(&copy).unwrap();
        assert_eq!(a, b);
        assert!(a.diagnostics.is_empty());
        packets += a.events.len();
    }
    assert!(packets >= 2612);
    assert_eq!(
        rec.counter(Instrument::DecodeBatchesTotal, &["replay", "binary", "ok"]),
        19
    );
    // Synthetic envelopes too, text included.
    let text = r#"{"type":"message","data":"hello"}"#.as_bytes().to_vec();
    for (kind, bytes) in [
        (PayloadKind::Text, text),
        (
            PayloadKind::Binary,
            read_ticker_fixture("protocol/single_quote.bin"),
        ),
        (PayloadKind::Binary, vec![1]),
    ] {
        let o = observe(&mut s, kind, bytes);
        assert_eq!(
            live.decode(&o).unwrap(),
            replay.decode(&captured(&o)).unwrap()
        );
    }
}

#[test]
fn bare_payloads_decode_without_any_source_identity() {
    let bytes = read_ticker_fixture("protocol/single_index_full.bin");
    let Message::Packets(frames) = frame(&bytes, FramingLimits::default()).unwrap() else {
        panic!()
    };
    let packets: Vec<Packet> = frames.iter().map(|f| decode(&f).unwrap()).collect();
    assert!(matches!(packets[..], [Packet::IndexFull(_)]));
}

#[test]
fn outputs_are_owned_and_unknown_envelope_versions_fail_explicitly() {
    fn owned<T: Send + Sync + 'static>() {}
    owned::<manja::kite::decoder::adapter::Decoded>();
    owned::<DecodedEvent>();
    let mut s = sequencer();
    let o = observe(&mut s, PayloadKind::Binary, vec![1]);
    let json = serde_json::to_string(&o)
        .unwrap()
        .replace("\"major\":1", "\"major\":2");
    assert!(serde_json::from_str::<RawObservation>(&json).is_err());
}

#[derive(Clone, Default)]
struct Names(Arc<Mutex<Vec<&'static str>>>);

impl<S: Subscriber> Layer<S> for Names {
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
        self.0.lock().unwrap().push(attrs.metadata().name());
    }
}

#[test]
fn one_count_per_observation_spans_off_by_default_and_no_source_label() {
    let rec = Arc::new(InMemoryRecorder::new());
    let obs = Observability::with_recorder(rec.clone());
    let names = Names::default();
    let _g = support::spans::set_default(tracing_subscriber::registry().with(names.clone()));
    let adapter = Adapter::new(SourceMode::Live, &obs);
    let mut s = sequencer();
    // ok, partial (one LTP and one 16-byte packet), error, unknown text,
    // invalid text.
    let mut partial = vec![0, 2, 0, 8];
    partial.extend(&read_ticker_fixture("protocol/single_ltp.bin")[4..]);
    partial.extend([0, 16]);
    partial.extend([0; 16]);
    let inputs = vec![
        (
            PayloadKind::Binary,
            read_ticker_fixture("protocol/single_full.bin"),
        ),
        (PayloadKind::Binary, partial),
        (
            PayloadKind::Binary,
            read_ticker_fixture("protocol/truncated.bin"),
        ),
        (PayloadKind::Text, br#"{"type":"future","data":1}"#.to_vec()),
        (PayloadKind::Text, b"not json".to_vec()),
    ];
    let observations: Vec<RawObservation> = inputs
        .into_iter()
        .map(|(k, b)| observe(&mut s, k, b))
        .collect();
    let quiet_out: Vec<_> = observations
        .iter()
        .map(|o| adapter.decode(o).unwrap())
        .collect();
    assert!(
        names.0.lock().unwrap().is_empty(),
        "decode spans are off by default"
    );
    assert_eq!(rec.counter_total(Instrument::DecodeBatchesTotal), 5);
    let per: BTreeMap<&str, u64> = [
        ("ok", &["live", "binary", "ok"][..]),
        ("partial", &["live", "binary", "partial"]),
        ("error", &["live", "binary", "error"]),
        ("unknown", &["live", "text", "unknown_format"]),
        ("text_error", &["live", "text", "error"]),
    ]
    .into_iter()
    .map(|(k, labels)| (k, rec.counter(Instrument::DecodeBatchesTotal, labels)))
    .collect();
    assert!(per.values().all(|n| *n == 1), "{per:?}");
    // With spans on, outputs are identical.
    let traced = adapter.clone().with_spans(true);
    let traced_out: Vec<_> = observations
        .iter()
        .map(|o| traced.decode(o).unwrap())
        .collect();
    assert_eq!(quiet_out, traced_out);
    assert_eq!(
        names
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|n| **n == "manja.decode.batch")
            .count(),
        5
    );
    for (labels, _) in rec.dump() {
        for v in labels.values() {
            assert!(!v.contains(FEED) && !v.contains("manja/"), "{v}");
        }
    }
}
