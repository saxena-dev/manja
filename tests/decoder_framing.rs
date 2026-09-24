//! Binary framing against the vendored corpus.
//!
//! The qdx `<case>.json` files (`docs/verification.md` §1.2) are a
//! cross-check, not protocol truth; the
//! dispositions of their differences are in `manja::kite::decoder::framing`.

mod support;

use manja::kite::decoder::framing::{
    frame, DecodeDiagnosticKind, FramingLimits, Message, PacketFamily,
};

use support::capture::{read_capture, read_real_capture, read_ticker_fixture};

fn oracle(name: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(
        support::capture::ticker_fixtures_dir().join(format!("protocol/{name}.json")),
    )
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

fn packets(payload: &[u8]) -> Vec<(u16, usize, PacketFamily)> {
    match frame(payload, FramingLimits::default()).unwrap() {
        Message::Packets(f) => f
            .iter()
            .map(|x| (x.index(), x.bytes().len(), x.family()))
            .collect(),
        Message::Heartbeat => panic!("a batch"),
    }
}

#[test]
fn the_heartbeat_is_recognized_before_quote_framing() {
    let bytes = read_ticker_fixture("protocol/heartbeat.bin");
    assert_eq!(oracle("heartbeat")["expected_type"], "Heartbeat");
    assert_eq!(bytes.len(), 1);
    assert_eq!(
        frame(&bytes, FramingLimits::default()),
        Ok(Message::Heartbeat)
    );
}

#[test]
fn valid_single_and_multi_packet_messages_frame_as_the_oracle_counts() {
    let cases = [
        ("single_ltp", PacketFamily::Ltp, 8),
        ("single_quote", PacketFamily::Quote, 44),
        ("single_full", PacketFamily::Full, 184),
        ("single_index_quote", PacketFamily::IndexQuote, 28),
        ("single_index_full", PacketFamily::IndexFull, 32),
        ("unknown_size", PacketFamily::Unknown, 16),
    ];
    for (name, family, len) in cases {
        let got = packets(&read_ticker_fixture(&format!("protocol/{name}.bin")));
        assert_eq!(got, [(0, len, family)], "{name}");
        assert_eq!(oracle(name)["expected_packets"], 1, "{name}");
    }
    let multi = packets(&read_ticker_fixture("protocol/multi_packet.bin"));
    assert_eq!(
        multi,
        [
            (0, 184, PacketFamily::Full),
            (1, 32, PacketFamily::IndexFull)
        ]
    );
    let o = oracle("multi_packet");
    assert_eq!(o["expected_packets"], 2);
    assert_eq!(o["packets"][0]["raw_len"], 184);
    assert_eq!(o["packets"][1]["raw_len"], 32);
}

#[test]
fn structurally_invalid_messages_expose_no_packet_and_leave_the_input_intact() {
    let cases = [
        ("malformed_count", DecodeDiagnosticKind::CountMismatch),
        ("truncated", DecodeDiagnosticKind::Truncated),
        // Disposition: qdx reports the packet too; the batch is rejected here.
        ("trailing_bytes", DecodeDiagnosticKind::TrailingBytes),
    ];
    for (name, kind) in cases {
        let bytes = read_ticker_fixture(&format!("protocol/{name}.bin"));
        let before = bytes.clone();
        let err = frame(&bytes, FramingLimits::default()).unwrap_err();
        assert_eq!(err.kind(), kind, "{name}");
        assert_eq!(bytes, before, "input unchanged and still usable: {name}");
        assert!(oracle(name)["expected_diagnostic"].is_string(), "{name}");
    }
    let trailing = read_ticker_fixture("protocol/trailing_bytes.bin");
    let err = frame(&trailing, FramingLimits::default()).unwrap_err();
    assert_eq!(
        err.offset(),
        trailing.len()
            - oracle("trailing_bytes")["trailing_byte_count"]
                .as_u64()
                .unwrap() as usize
    );
}

#[test]
fn every_record_of_the_real_capture_frames_cleanly() {
    let capture = read_real_capture();
    let records = read_capture(&capture).unwrap();
    assert_eq!(records.len(), 19);
    let mut total = 0;
    for (i, r) in records.iter().enumerate() {
        match frame(r.payload, FramingLimits::default()) {
            Ok(Message::Packets(f)) => {
                for p in f.iter() {
                    assert!(
                        matches!(p.family(), PacketFamily::Full | PacketFamily::IndexFull),
                        "record {i} packet {}: {:?}",
                        p.index(),
                        p.family()
                    );
                }
                total += f.len();
            }
            Ok(Message::Heartbeat) => {}
            Err(e) => panic!("record {i}: {e}"),
        }
    }
    assert!(total >= 2612, "{total}");
}
