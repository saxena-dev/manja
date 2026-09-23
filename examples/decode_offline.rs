//! Offline decoding of captured ticker bytes, with no runtime, network,
//! credential or storage: `cargo run --example decode_offline
//! --no-default-features --features decoder`.
//!
//! It reads the test-only capture container in `tests/fixtures/ticker/`
//! (records of a little-endian u64 receipt time, a little-endian u32 length
//! and the unchanged Kite message), frames and decodes every message, and
//! wraps each one in a source envelope to show provenance.

use manja::kite::decoder::adapter::Adapter;
use manja::kite::decoder::framing::{frame, FramingLimits, Message};
use manja::kite::decoder::packets::{decode, Packet};
use manja::kite::envelope::{
    MonotonicElapsed, PayloadKind, RawObservation, ReceiveTime, SourceIdentity, SourceSequencer,
};
use manja::kite::obs::schema::SourceMode;
use manja::kite::obs::Observability;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ticker/tick_set_0__2026_01_21.bin"
    );
    let capture = std::fs::read(path)?;
    let mut rest = &capture[..];
    let mut sequencer = SourceSequencer::new(SourceIdentity::generate());
    sequencer.begin_epoch();
    let adapter = Adapter::new(SourceMode::Replay, &Observability::disabled());
    let (mut messages, mut full, mut index) = (0, 0, 0);
    while rest.len() >= 12 {
        let received = u64::from_le_bytes(rest[..8].try_into()?);
        let len = u32::from_le_bytes(rest[8..12].try_into()?) as usize;
        let payload = &rest[12..12 + len];
        rest = &rest[12 + len..];
        messages += 1;

        // Bare parsing: no source identity needed.
        if let Message::Packets(frames) = frame(payload, FramingLimits::default())? {
            for f in frames.iter() {
                match decode(&f)? {
                    Packet::Full(_) => full += 1,
                    Packet::IndexFull(_) => index += 1,
                    _ => {}
                }
            }
        }

        // With provenance: every event carries the observation's source key.
        let observation = RawObservation::new(
            sequencer.next_key(),
            PayloadKind::Binary,
            ReceiveTime::from_unix_nanos(received as i64),
            MonotonicElapsed::from_nanos(0),
            payload.to_vec(),
            16 << 20,
        )?;
        let decoded = adapter.decode(&observation)?;
        assert!(decoded.diagnostics.is_empty());
    }
    println!("{messages} messages: {full} full packets, {index} index packets");
    Ok(())
}
