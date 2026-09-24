//! Wrap every record of the vendored real capture (manifest entry
//! `tick_set_0__2026_01_21.bin`, SHA-256 a54f73d1…cb912) in raw-observation
//! envelopes and check that bytes and receive times survive unchanged. Needs
//! no async runtime or feature.

#[allow(dead_code)]
#[path = "support/capture.rs"]
mod capture;

use manja::kite::envelope::{
    MonotonicElapsed, PayloadKind, RawObservation, ReceiveTime, SourceIdentity, SourceSequencer,
    DEFAULT_MAX_PAYLOAD_BYTES,
};

#[test]
fn real_capture_payloads_and_receive_times_are_preserved() {
    let bytes = capture::read_real_capture();
    let records = capture::read_capture(&bytes).unwrap();
    let mut seq = SourceSequencer::new(SourceIdentity::generate());
    let epoch = seq.begin_epoch();
    for (i, record) in records.iter().enumerate() {
        let obs = RawObservation::new(
            seq.next_key(),
            PayloadKind::Binary,
            ReceiveTime::from_unix_nanos(record.receipt_unix_nanos as i64),
            MonotonicElapsed::from_nanos(i as u64),
            record.payload.to_vec(),
            DEFAULT_MAX_PAYLOAD_BYTES,
        )
        .unwrap();
        assert_eq!(obs.payload().as_bytes(), record.payload, "record {i}");
        assert_eq!(
            obs.received_at().unix_nanos(),
            record.receipt_unix_nanos as i64
        );
        assert_eq!(obs.source().connection_epoch(), epoch);
        assert_eq!(obs.source().ingress_sequence(), i as u64);
    }
    assert_eq!(records.len(), 19);
}

#[test]
fn vendored_heartbeat_is_an_ordinary_binary_observation() {
    let heartbeat = capture::read_ticker_fixture("protocol/heartbeat.bin");
    assert_eq!(heartbeat, [0x01]);
    let mut seq = SourceSequencer::new(SourceIdentity::generate());
    seq.begin_epoch();
    let obs = RawObservation::new(
        seq.next_key(),
        PayloadKind::Binary,
        ReceiveTime::now(),
        seq.elapsed_nanos_now(),
        heartbeat.clone(),
        DEFAULT_MAX_PAYLOAD_BYTES,
    )
    .unwrap();
    assert_eq!(obs.payload().as_bytes(), heartbeat.as_slice());
}
