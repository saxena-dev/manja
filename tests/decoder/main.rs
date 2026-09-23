//! Decoder qualification: the golden corpus mapped to the coverage
//! inventory (`docs/verification.md` §2), and a seeded, budgeted property
//! campaign (`docs/verification.md` §4).
//!
//! The campaign uses an in-crate SplitMix64 generator: no fuzzing crate is
//! added. Every case derives from one recorded seed, so a failure replays
//! exactly. Override the seed and the budget multiplier with
//! `MANJA_FUZZ_SEED` (hex or decimal) and `MANJA_FUZZ_SCALE` (default 1):
//!
//! ```text
//! MANJA_FUZZ_SEED=0x5eed2026 MANJA_FUZZ_SCALE=10 \
//!   cargo test --no-default-features --features decoder --test decoder_qualification -- --nocapture
//! ```
//!
//! A finite campaign is evidence, never proof against all inputs.

#[path = "../support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use manja::kite::decoder::adapter::{Adapter, DecodedEvent, VERSIONS};
use manja::kite::decoder::framing::{
    frame, DecodeDiagnosticKind, FramingLimits, Message, PacketFamily,
};
use manja::kite::decoder::packets::{decode_bytes, scaled, Packet, PacketErrorKind};
use manja::kite::decoder::text::{parse, TextEvent, TextLimits};
use manja::kite::envelope::{
    MonotonicElapsed, PayloadKind, RawObservation, ReceiveTime, RunId, SourceIdentity,
    SourceSequencer,
};
use manja::kite::obs::schema::SourceMode;
use manja::kite::obs::Observability;
use manja::kite::protocol::scale::Segment;

use support::capture::{read_capture, read_real_capture, read_ticker_fixture};
use support::fixtures;

// ---- generator --------------------------------------------------------

const DEFAULT_SEED: u64 = 0x5eed_2026;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
    fn extreme_i32(&mut self) -> i32 {
        match self.below(6) {
            0 => i32::MIN,
            1 => i32::MAX,
            2 => 0,
            3 => -1,
            4 => 1,
            _ => self.next() as i32,
        }
    }
}

fn seed() -> u64 {
    match std::env::var("MANJA_FUZZ_SEED") {
        Ok(s) => {
            let s = s.trim();
            match s.strip_prefix("0x") {
                Some(h) => u64::from_str_radix(h, 16).expect("hex seed"),
                None => s.parse().expect("decimal seed"),
            }
        }
        Err(_) => DEFAULT_SEED,
    }
}

fn budget(base: u64) -> u64 {
    let scale: u64 = std::env::var("MANJA_FUZZ_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    base * scale
}

fn campaign(name: &str, base: u64, salt: u64) -> (Rng, u64) {
    let (s, n) = (seed(), budget(base));
    println!("campaign {name}: seed {s:#x}, salt {salt:#x}, {n} cases");
    (Rng(s ^ salt), n)
}

// ---- reference outputs ------------------------------------------------

fn batch(packets: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (packets.len() as u16).to_be_bytes().to_vec();
    for p in packets {
        out.extend((p.len() as u16).to_be_bytes());
        out.extend(p);
    }
    out
}

// Re-encode a decoded packet at its documented offsets. Depth padding is not
// carried, so it is written as zero and masked on comparison.
fn encode(p: &Packet) -> Vec<u8> {
    fn put(out: &mut Vec<u8>, v: i32) {
        out.extend(v.to_be_bytes());
    }
    let mut out = Vec::new();
    match p {
        Packet::Ltp(l) => {
            out.extend(l.instrument_token.get().to_be_bytes());
            put(&mut out, l.last_price);
        }
        Packet::Quote(q) => quote(&mut out, &q.fields),
        Packet::Full(f) => {
            quote(&mut out, &f.fields);
            for v in [
                f.last_trade_time,
                f.open_interest,
                f.open_interest_day_high,
                f.open_interest_day_low,
                f.exchange_timestamp,
            ] {
                out.extend(v.to_be_bytes());
            }
            for e in f.depth.bids.iter().chain(&f.depth.offers) {
                out.extend(e.quantity.to_be_bytes());
                put(&mut out, e.price);
                out.extend(e.orders.to_be_bytes());
                out.extend([0, 0]);
            }
        }
        Packet::IndexQuote(i) => index(&mut out, &i.fields),
        Packet::IndexFull(i) => {
            index(&mut out, &i.fields);
            out.extend(i.exchange_timestamp.to_be_bytes());
        }
        _ => panic!("unexpected family"),
    }
    fn quote(out: &mut Vec<u8>, q: &manja::kite::decoder::packets::QuoteFields) {
        out.extend(q.instrument_token.get().to_be_bytes());
        out.extend(q.last_price.to_be_bytes());
        out.extend(q.last_quantity.to_be_bytes());
        out.extend(q.average_price.to_be_bytes());
        for v in [q.volume, q.buy_quantity, q.sell_quantity] {
            out.extend(v.to_be_bytes());
        }
        for v in [q.open, q.high, q.low, q.close] {
            out.extend(v.to_be_bytes());
        }
    }
    fn index(out: &mut Vec<u8>, i: &manja::kite::decoder::packets::IndexFields) {
        out.extend(i.instrument_token.get().to_be_bytes());
        for v in [i.last_price, i.high, i.low, i.open, i.close, i.change] {
            out.extend(v.to_be_bytes());
        }
    }
    out
}

fn mask_padding(bytes: &[u8]) -> Vec<u8> {
    let mut b = bytes.to_vec();
    if b.len() == 184 {
        for i in 0..10 {
            let at = 64 + 12 * i + 10;
            b[at] = 0;
            b[at + 1] = 0;
        }
    }
    b
}

// A successful decode must be exactly the bytes it came from: no plausible
// tick from an invalid input.
fn assert_faithful(bytes: &[u8]) {
    match decode_bytes(bytes) {
        Ok(p) => assert_eq!(encode(&p), mask_padding(bytes), "decode is not faithful"),
        Err(PacketErrorKind::UnknownLength(n)) => {
            assert_eq!(n, bytes.len());
            assert_eq!(PacketFamily::of_len(n), PacketFamily::Unknown);
        }
        Err(PacketErrorKind::NegativeQuantity { offset }) => {
            assert!(offset + 2 <= bytes.len());
            assert!(bytes[offset] & 0x80 != 0, "only a set sign bit is refused");
        }
        Err(other) => panic!("{other:?}"),
    }
}

// A framed batch must account for every byte exactly.
fn assert_frames_exact(
    payload: &[u8],
    limits: FramingLimits,
) -> Result<usize, DecodeDiagnosticKind> {
    match frame(payload, limits) {
        Ok(Message::Heartbeat) => {
            assert_eq!(payload.len(), 1);
            Ok(0)
        }
        Ok(Message::Packets(frames)) => {
            assert!(frames.len() <= limits.max_packets() as usize);
            let mut total = 2;
            for f in frames.iter() {
                assert!(!f.bytes().is_empty());
                total += 2 + f.bytes().len();
                assert_faithful(f.bytes());
            }
            assert_eq!(total, payload.len(), "a batch frames exactly");
            Ok(frames.len())
        }
        Err(e) => Err(e.kind()),
    }
}

// ---- golden corpus mapped to the coverage inventory --------------------

fn packets_of(name: &str) -> Vec<Packet> {
    let bytes = read_ticker_fixture(&format!("protocol/{name}.bin"));
    match frame(&bytes, FramingLimits::default()).unwrap() {
        Message::Packets(f) => f.iter().map(|x| decode_bytes(x.bytes()).unwrap()).collect(),
        Message::Heartbeat => vec![],
    }
}

fn base64(text: &str) -> Vec<u8> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0);
    for c in text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
    {
        acc = (acc << 6) | alphabet.iter().position(|a| *a == c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

#[test]
fn golden_packet_families_inv_b() {
    // INV-B-01 LTP: qdx single_ltp (no official packet exists, INV-GAP-05).
    assert!(matches!(packets_of("single_ltp")[..], [Packet::Ltp(_)]));
    // INV-B-02 tradable quote: official ticker_quote.packet and qdx single_quote.
    let official = base64(&fixtures::read("ticker_quote.packet").unwrap());
    assert!(matches!(decode_bytes(&official), Ok(Packet::Quote(_))));
    assert!(matches!(packets_of("single_quote")[..], [Packet::Quote(_)]));
    // INV-B-03 full with depth: official, qdx single_full, and the capture.
    let official = base64(&fixtures::read("ticker_full.packet").unwrap());
    let Ok(Packet::Full(full)) = decode_bytes(&official) else {
        panic!()
    };
    assert_eq!((full.depth.bids.len(), full.depth.offers.len()), (5, 5));
    // Timestamps are the raw exchange seconds (official JSON: 2021-07-05
    // 10:41:27 +05:30).
    assert_eq!(full.exchange_timestamp, 1_625_461_887);
    assert_faithful(&official);
    // INV-B-04 index quote, INV-B-05 index full.
    assert!(matches!(
        packets_of("single_index_quote")[..],
        [Packet::IndexQuote(_)]
    ));
    assert!(matches!(
        packets_of("single_index_full")[..],
        [Packet::IndexFull(_)]
    ));
    // INV-B-06 heartbeat: a message, not a tick.
    assert_eq!(
        frame(
            &read_ticker_fixture("protocol/heartbeat.bin"),
            FramingLimits::default()
        ),
        Ok(Message::Heartbeat)
    );
}

#[test]
fn golden_framing_cases_inv_f() {
    let l = FramingLimits::default();
    // INV-F-01 batch: qdx multi_packet and all 19 capture records.
    assert_eq!(
        assert_frames_exact(&read_ticker_fixture("protocol/multi_packet.bin"), l),
        Ok(2)
    );
    let capture = read_real_capture();
    let records = read_capture(&capture).unwrap();
    assert_eq!(records.len(), 19);
    let packets: usize = records
        .iter()
        .map(|r| assert_frames_exact(r.payload, l).unwrap())
        .sum();
    assert_eq!(packets, 2692);
    // INV-F-02..05.
    for (name, kind) in [
        ("malformed_count", DecodeDiagnosticKind::CountMismatch),
        ("truncated", DecodeDiagnosticKind::Truncated),
        ("trailing_bytes", DecodeDiagnosticKind::TrailingBytes),
    ] {
        assert_eq!(
            assert_frames_exact(&read_ticker_fixture(&format!("protocol/{name}.bin")), l),
            Err(kind),
            "{name}"
        );
    }
    let unknown = read_ticker_fixture("protocol/unknown_size.bin");
    assert_eq!(assert_frames_exact(&unknown, l), Ok(1));
}

#[test]
fn golden_text_scale_and_provenance() {
    let l = TextLimits::default();
    // INV-T-01 error and message text (labelled supplements).
    assert!(matches!(
        parse(r#"{"type":"error","data":"x"}"#, l),
        Ok(TextEvent::Error(_))
    ));
    assert!(matches!(
        parse(r#"{"type":"message","data":"x"}"#, l),
        Ok(TextEvent::Message(_))
    ));
    // INV-T-02 order update: official postback.json as order data.
    let postback = fixtures::json_body("postback.json").unwrap();
    let text = format!("{{\"type\":\"order\",\"data\":{postback}}}");
    assert!(matches!(parse(&text, l), Ok(TextEvent::Order(_))));
    // INV-T-03 unknown text variant (labelled supplement).
    assert!(matches!(
        parse(r#"{"type":"zzz","data":{}}"#, l),
        Ok(TextEvent::Unknown { .. })
    ));
    // INV-S-01 CDS divides by 10^7; INV-S-02 BCD is refused.
    assert_eq!(scaled(831_234_000, Segment::Cds).unwrap().to_f64(), 83.1234);
    assert!(scaled(1, Segment::Bcd).is_err());
    // Provenance: every event carries the source key and pinned versions.
    let mut s = SourceSequencer::new(SourceIdentity::generate());
    s.begin_epoch();
    let o = RawObservation::new(
        s.next_key(),
        PayloadKind::Binary,
        ReceiveTime::from_unix_nanos(0),
        MonotonicElapsed::from_nanos(0),
        read_ticker_fixture("protocol/multi_packet.bin"),
        1 << 20,
    )
    .unwrap();
    let d = Adapter::new(SourceMode::Standalone, &Observability::disabled())
        .decode(&o)
        .unwrap();
    assert_eq!(d.versions, VERSIONS);
    assert!(d
        .events
        .iter()
        .all(|e| matches!(e, DecodedEvent::Packet { source, .. } if source == o.source())));
}

#[test]
fn a_mutated_vendored_byte_is_caught() {
    let mut bytes = read_ticker_fixture("protocol/single_full.bin");
    let before = packets_of("single_full");
    // Byte 13 is inside the last traded price of the one packet.
    bytes[13] ^= 0x01;
    let Message::Packets(f) = frame(&bytes, FramingLimits::default()).unwrap() else {
        panic!()
    };
    let after: Vec<Packet> = f.iter().map(|x| decode_bytes(x.bytes()).unwrap()).collect();
    assert_ne!(after, before, "the regression suite detects a changed byte");
}

// ---- raw bytes and clock independence ----------------------------------

#[test]
fn outputs_ignore_clocks_and_leave_raw_bytes_intact() {
    let capture = read_real_capture();
    let identity = SourceIdentity::new("manja", RunId::from_u128(9), "qual").unwrap();
    let adapter = Adapter::new(SourceMode::Replay, &Observability::disabled());
    for r in read_capture(&capture).unwrap() {
        let make = |t: i64, e: u64| {
            let mut s = SourceSequencer::new(identity.clone());
            s.begin_epoch();
            RawObservation::new(
                s.next_key(),
                PayloadKind::Binary,
                ReceiveTime::from_unix_nanos(t),
                MonotonicElapsed::from_nanos(e),
                r.payload.to_vec(),
                16 << 20,
            )
            .unwrap()
        };
        // Different wall and monotonic times, as a shifted clock would give.
        let (a, b) = (make(0, 0), make(r.receipt_unix_nanos as i64, 1 << 40));
        assert_eq!(adapter.decode(&a).unwrap(), adapter.decode(&b).unwrap());
        // A captured copy decodes identically.
        let copy: RawObservation =
            serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(adapter.decode(&a).unwrap(), adapter.decode(&copy).unwrap());
    }
    // After a failure the raw bytes are still readable and unchanged.
    let bad = read_ticker_fixture("protocol/truncated.bin");
    let mut s = SourceSequencer::new(identity);
    s.begin_epoch();
    let o = RawObservation::new(
        s.next_key(),
        PayloadKind::Binary,
        ReceiveTime::from_unix_nanos(0),
        MonotonicElapsed::from_nanos(0),
        bad.clone(),
        1 << 20,
    )
    .unwrap();
    assert!(!adapter.decode(&o).unwrap().diagnostics.is_empty());
    assert_eq!(o.payload().as_bytes(), bad);
}

// ---- seeded property campaign ------------------------------------------

const FAMILIES: [usize; 5] = [8, 28, 32, 44, 184];

#[test]
fn campaign_count_and_length_framing() {
    let (mut rng, n) = campaign("framing", 20_000, 0x01);
    let limits = FramingLimits::default();
    for _ in 0..n {
        let payload = match rng.below(3) {
            // Arbitrary bytes.
            0 => {
                let len = rng.below(600) as usize;
                rng.bytes(len)
            }
            // A plausible header with random count and lengths.
            1 => {
                let mut p = (rng.below(8) as u16).to_be_bytes().to_vec();
                for _ in 0..rng.below(8) {
                    let len = if rng.below(2) == 0 {
                        FAMILIES[rng.below(5) as usize]
                    } else {
                        rng.below(300) as usize
                    };
                    p.extend((len as u16).to_be_bytes());
                    let body = rng.below(len as u64 + 3) as usize;
                    p.extend(rng.bytes(body));
                }
                p
            }
            // A valid batch of random packets of documented lengths.
            _ => {
                let packets: Vec<Vec<u8>> = (0..rng.below(6))
                    .map(|_| {
                        let len = FAMILIES[rng.below(5) as usize];
                        rng.bytes(len)
                    })
                    .collect();
                let p = batch(&packets);
                assert_eq!(assert_frames_exact(&p, limits), Ok(packets.len()));
                p
            }
        };
        let _ = assert_frames_exact(&payload, limits);
    }
}

#[test]
fn campaign_trailing_bytes() {
    let (mut rng, n) = campaign("trailing", 10_000, 0x02);
    for _ in 0..n {
        let packets: Vec<Vec<u8>> = (0..1 + rng.below(4))
            .map(|_| {
                let len = FAMILIES[rng.below(5) as usize];
                rng.bytes(len)
            })
            .collect();
        let mut p = batch(&packets);
        let extra = 1 + rng.below(40) as usize;
        p.extend(rng.bytes(extra));
        assert_eq!(
            assert_frames_exact(&p, FramingLimits::default()),
            Err(DecodeDiagnosticKind::TrailingBytes)
        );
    }
}

#[test]
fn campaign_allocation_and_work_bounds() {
    let (mut rng, n) = campaign("bounds", 200, 0x03);
    let limits = FramingLimits::default().with_max_payload(16 << 20).unwrap();
    for _ in 0..n {
        // Huge declared counts over small or large buffers.
        let len = [3usize, 1 << 10, 1 << 20, 8 << 20][rng.below(4) as usize];
        let mut p = rng.bytes(len);
        p[0] = 0xFF;
        p[1] = rng.next() as u8;
        let start = Instant::now();
        let r = frame(&p, limits);
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "linear, bounded work"
        );
        if let Err(e) = r {
            assert!(matches!(
                e.kind(),
                DecodeDiagnosticKind::CountMismatch
                    | DecodeDiagnosticKind::Truncated
                    | DecodeDiagnosticKind::TrailingBytes
                    | DecodeDiagnosticKind::UnknownLength
            ));
        }
    }
    // Over the payload bound: refused before any work.
    let big = vec![0u8; (16 << 20) + 1];
    assert_eq!(
        frame(&big, limits).unwrap_err().kind(),
        DecodeDiagnosticKind::Oversized
    );
}

#[test]
fn campaign_malformed_text() {
    let (mut rng, n) = campaign("text", 20_000, 0x04);
    let seeds = [
        r#"{"type":"order","data":{"order_id":"1","status":"COMPLETE"}}"#.to_string(),
        r#"{"type":"error","data":"x"}"#.to_string(),
        r#"{"type":"message","data":"y"}"#.to_string(),
        format!(
            "{{\"type\":\"order\",\"data\":{}}}",
            fixtures::json_body("postback.json").unwrap()
        ),
    ];
    let l = TextLimits::default();
    for _ in 0..n {
        let mut bytes = seeds[rng.below(seeds.len() as u64) as usize]
            .clone()
            .into_bytes();
        for _ in 0..1 + rng.below(4) {
            match rng.below(4) {
                0 if !bytes.is_empty() => {
                    let i = rng.below(bytes.len() as u64) as usize;
                    let alphabet = b"{}[]\":,\\0a ";
                    bytes[i] = alphabet[rng.below(alphabet.len() as u64) as usize];
                }
                1 if !bytes.is_empty() => {
                    let cut = rng.below(bytes.len() as u64) as usize;
                    bytes.truncate(cut);
                }
                2 => {
                    let depth = rng.below(80) as usize;
                    bytes = format!("{}{}", "[".repeat(depth), String::from_utf8_lossy(&bytes))
                        .into_bytes();
                }
                _ => {
                    let k = rng.below(8) as usize;
                    bytes.extend(rng.bytes(k))
                }
            }
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        match parse(text, l) {
            Ok(event) => {
                // Success only for a JSON object with a string type.
                let v: serde_json::Value = serde_json::from_str(text).unwrap();
                let t = v["type"].as_str().expect("a string type");
                match event {
                    TextEvent::Order(u) => assert_eq!(t, "order", "{}", u.order_id),
                    TextEvent::Error(_) => assert_eq!(t, "error"),
                    TextEvent::Message(_) => assert_eq!(t, "message"),
                    TextEvent::Unknown { message_type, .. } => {
                        assert!(!["order", "error", "message"].contains(&t));
                        assert!(message_type.as_str().len() <= 512);
                    }
                    _ => {}
                }
            }
            Err(e) => assert!(e.detail().as_str().len() <= 512),
        }
    }
}

#[test]
fn campaign_numeric_extremes_and_segments() {
    let (mut rng, n) = campaign("numeric", 20_000, 0x05);
    for _ in 0..n {
        let len = FAMILIES[rng.below(5) as usize];
        let mut p = Vec::with_capacity(len);
        while p.len() < len {
            p.extend(rng.extreme_i32().to_be_bytes());
        }
        p.truncate(len);
        assert_faithful(&p);
    }
    let segments = [
        Segment::Nse,
        Segment::Nfo,
        Segment::Cds,
        Segment::Bse,
        Segment::Bfo,
        Segment::Bcd,
        Segment::Mcx,
        Segment::McxSx,
        Segment::Indices,
    ];
    for _ in 0..n {
        let raw = rng.extreme_i32();
        for s in segments {
            match scaled(raw, s) {
                Ok(p) => assert_eq!(p.raw(), raw as i64),
                Err(_) => assert_eq!(s, Segment::Bcd, "only the unverified segment is refused"),
            }
        }
    }
}

#[test]
fn campaign_capture_mutations() {
    let (mut rng, n) = campaign("capture", 3_000, 0x06);
    let capture = read_real_capture();
    let records = read_capture(&capture).unwrap();
    let limits = FramingLimits::default();
    for _ in 0..n {
        let r = &records[rng.below(records.len() as u64) as usize];
        let mut p = r.payload.to_vec();
        for _ in 0..1 + rng.below(3) {
            match rng.below(4) {
                0 => {
                    let i = rng.below(p.len() as u64) as usize;
                    p[i] ^= 1 << rng.below(8);
                }
                1 => {
                    let cut = rng.below(p.len() as u64) as usize;
                    p.truncate(cut.max(1));
                }
                2 => {
                    let at = rng.below(p.len() as u64) as usize;
                    let k = 1 + rng.below(8) as usize;
                    let extra = rng.bytes(k);
                    p.splice(at..at, extra);
                }
                _ => {
                    let k = 1 + rng.below(8) as usize;
                    p.extend(rng.bytes(k))
                }
            }
        }
        let _ = assert_frames_exact(&p, limits);
    }
}
