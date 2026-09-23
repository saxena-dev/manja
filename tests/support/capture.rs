// Reader for the test-only capture container of the vendored ticker corpus.
//
// `tests/fixtures/ticker/tick_set_0__2026_01_21.bin` is a sequence of records,
// each a little-endian `u64` receipt time in Unix nanoseconds, a little-endian
// `u32` payload length, then that many bytes of an unchanged big-endian Kite
// binary WebSocket message. This container is a TEST INPUT FORMAT ONLY: it is
// not part of manja's public API, not a manja format, and not a persistent
// capture-log format. The reader exists only in test support code.
//
// This file holds only items and depends only on `std`, so both the fixture
// manifest test and the decoder tests can include it with `#[path]`.

use std::fmt;
use std::path::PathBuf;

/// One record of the capture container.
pub struct CaptureRecord<'a> {
    /// Receipt time recorded by the capturing process, Unix nanoseconds.
    pub receipt_unix_nanos: u64,
    /// The Kite binary message exactly as received.
    pub payload: &'a [u8],
}

/// Why the container could not be framed.
#[derive(Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// A record header was cut off at `offset`.
    TruncatedHeader { offset: usize },
    /// A record declared `declared` payload bytes but only `available` remain.
    TruncatedPayload {
        offset: usize,
        declared: usize,
        available: usize,
    },
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Split `bytes` into capture records. Fails on any truncation; trailing bytes
/// cannot exist because every byte belongs to a header or a payload.
pub fn read_capture(bytes: &[u8]) -> Result<Vec<CaptureRecord<'_>>, CaptureError> {
    const HEADER: usize = 8 + 4;
    let mut records = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + HEADER)
            .ok_or(CaptureError::TruncatedHeader { offset })?;
        let mut nanos = [0u8; 8];
        nanos.copy_from_slice(&header[..8]);
        let mut len = [0u8; 4];
        len.copy_from_slice(&header[8..]);
        let declared = u32::from_le_bytes(len) as usize;
        let start = offset + HEADER;
        let payload = bytes
            .get(start..start + declared)
            .ok_or(CaptureError::TruncatedPayload {
                offset,
                declared,
                available: bytes.len() - start,
            })?;
        records.push(CaptureRecord {
            receipt_unix_nanos: u64::from_le_bytes(nanos),
            payload,
        });
        offset = start + declared;
    }
    Ok(records)
}

/// Directory of the vendored binary ticker corpus.
pub fn ticker_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ticker")
}

/// Read a vendored ticker fixture by its manifest path, e.g. `protocol/single_full.bin`.
pub fn read_ticker_fixture(path: &str) -> Vec<u8> {
    let full = ticker_fixtures_dir().join(path);
    std::fs::read(&full).unwrap_or_else(|e| panic!("cannot read {}: {e}", full.display()))
}

/// The real capture, `tick_set_0__2026_01_21.bin`.
pub fn read_real_capture() -> Vec<u8> {
    read_ticker_fixture("tick_set_0__2026_01_21.bin")
}
