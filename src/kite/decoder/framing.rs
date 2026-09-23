//! Binary message framing (`kite-api-docs/docs/connect/v3/websocket.md:62-80`).
//!
//! A binary message is either the one-byte heartbeat (`websocket.md:62`) or
//! a batch: a big-endian `u16` packet count, then for each packet a
//! big-endian `u16` length and that many packet bytes.
//!
//! [`frame`] validates the whole batch before exposing anything: the count
//! against its bound (`B-DEC-02`) and against the bytes available, every
//! length prefix, the arithmetic, and that no byte remains after the last
//! packet. A structurally invalid batch yields a [`FramingError`] and no
//! packets at all. Validation walks the length prefixes once without
//! allocating; iteration then yields borrowed [`Frame`]s.
//!
//! Framing does not interpret packet bytes. A packet whose length no
//! documented family uses is framed like any other and reported as unknown
//! by [`Frame::family`]; a zero-length packet is impossible and rejected.
//!
//! The parser is pure: no clock, allocation per packet, randomness,
//! telemetry, credential or runtime state takes part, and the input is only
//! borrowed, so it is unchanged and usable after any error.
//!
//! # Oracle dispositions
//!
//! The vendored qdx expectations are a cross-implementation oracle, not
//! protocol truth. Where they differ from the behavior above:
//!
//! - `trailing_bytes`: qdx reports the one packet and a `TrailingBytes`
//!   diagnostic. Here the batch is rejected with
//!   [`DecodeDiagnosticKind::TrailingBytes`] and no packet is exposed,
//!   because a batch that does not frame exactly is structurally invalid.
//! - `malformed_count`: qdx's `InvalidPacketCount` corresponds to
//!   [`DecodeDiagnosticKind::CountMismatch`].
//! - `unknown_size`: qdx exposes one packet of unknown type; here it frames
//!   as one packet whose [`Frame::family`] is [`PacketFamily::Unknown`].
//! - `heartbeat.bin` is the single byte `0x01`. The documentation gives the
//!   heartbeat's length only, not its value, so every one-byte binary
//!   message is a heartbeat; there is no documented one-byte non-heartbeat.
//!
use std::fmt;

pub use crate::kite::obs::diagnostics::DecodeDiagnosticKind;

/// Default `B-DEC-01`: input payload bytes, 1 MiB.
pub const DEFAULT_MAX_PAYLOAD: usize = 1 << 20;
/// Default `B-DEC-02`: packets per binary message.
pub const DEFAULT_MAX_PACKETS: u16 = 4096;

/// A decoder bound outside its documented range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecoderLimitError(pub &'static str);

impl fmt::Display for DecoderLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is outside its documented range", self.0)
    }
}

impl std::error::Error for DecoderLimitError {}

/// Framing bounds (SDK contract §5.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramingLimits {
    max_payload: usize,
    max_packets: u16,
}

impl Default for FramingLimits {
    fn default() -> Self {
        Self {
            max_payload: DEFAULT_MAX_PAYLOAD,
            max_packets: DEFAULT_MAX_PACKETS,
        }
    }
}

impl FramingLimits {
    /// Input payload bytes (`B-DEC-01`, 64 KiB to 16 MiB).
    pub fn with_max_payload(mut self, n: usize) -> Result<Self, DecoderLimitError> {
        if !(64 << 10..=16 << 20).contains(&n) {
            return Err(DecoderLimitError("B-DEC-01"));
        }
        self.max_payload = n;
        Ok(self)
    }

    /// Packets per message (`B-DEC-02`, 1 to 65 535).
    pub fn with_max_packets(mut self, n: u16) -> Result<Self, DecoderLimitError> {
        if n == 0 {
            return Err(DecoderLimitError("B-DEC-02"));
        }
        self.max_packets = n;
        Ok(self)
    }

    /// `B-DEC-01`.
    pub fn max_payload(&self) -> usize {
        self.max_payload
    }

    /// `B-DEC-02`.
    pub fn max_packets(&self) -> u16 {
        self.max_packets
    }
}

/// Why a binary message does not frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramingError {
    kind: DecodeDiagnosticKind,
    offset: usize,
    packet_index: Option<u16>,
}

impl FramingError {
    /// The diagnostic kind.
    pub fn kind(&self) -> DecodeDiagnosticKind {
        self.kind
    }

    /// Byte offset at which the problem was found.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The packet concerned, if any.
    pub fn packet_index(&self) -> Option<u16> {
        self.packet_index
    }
}

impl fmt::Display for FramingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "binary message framing: {} at byte {}",
            self.kind.as_str(),
            self.offset
        )?;
        if let Some(i) = self.packet_index {
            write!(f, " (packet {i})")?;
        }
        Ok(())
    }
}

impl std::error::Error for FramingError {}

/// The packet family implied by a packet's length
/// (`websocket.md:84-163`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PacketFamily {
    /// 8 bytes: LTP.
    Ltp,
    /// 28 bytes: index quote.
    IndexQuote,
    /// 32 bytes: index full.
    IndexFull,
    /// 44 bytes: quote.
    Quote,
    /// 184 bytes: full, with market depth.
    Full,
    /// Any other length.
    Unknown,
}

impl PacketFamily {
    /// The family of a packet of `len` bytes.
    pub const fn of_len(len: usize) -> Self {
        match len {
            8 => Self::Ltp,
            28 => Self::IndexQuote,
            32 => Self::IndexFull,
            44 => Self::Quote,
            184 => Self::Full,
            _ => Self::Unknown,
        }
    }
}

/// One framed packet, borrowed from the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    index: u16,
    bytes: &'a [u8],
}

impl<'a> Frame<'a> {
    /// Position within the message, from 0.
    pub fn index(&self) -> u16 {
        self.index
    }

    /// The packet bytes, excluding the length prefix.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The family implied by the length.
    pub fn family(&self) -> PacketFamily {
        PacketFamily::of_len(self.bytes.len())
    }
}

/// A validated batch. It borrows the message; its frames live as long as
/// the message bytes, and cannot outlive them:
///
/// ```compile_fail
/// use manja::kite::decoder::framing::{frame, FramingLimits, Message};
/// let kept = {
///     let payload = vec![0, 1, 0, 8, 0, 3, 0xE9, 9, 0, 0x22, 0x41, 0x88];
///     match frame(&payload, FramingLimits::default()).unwrap() {
///         Message::Packets(f) => f.iter().next().unwrap(),
///         Message::Heartbeat => unreachable!(),
///     }
/// };
/// let _ = kept.bytes();
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frames<'a> {
    // The bytes after the count, known to frame exactly.
    body: &'a [u8],
    count: u16,
}

impl<'a> Frames<'a> {
    /// Number of packets.
    pub fn len(&self) -> usize {
        self.count as usize
    }

    /// Whether the batch has no packets.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The packets, in order.
    pub fn iter(&self) -> FrameIter<'a> {
        FrameIter {
            rest: self.body,
            next: 0,
            count: self.count,
        }
    }
}

impl<'a> IntoIterator for Frames<'a> {
    type Item = Frame<'a>;
    type IntoIter = FrameIter<'a>;

    fn into_iter(self) -> FrameIter<'a> {
        self.iter()
    }
}

/// Iterator over the frames of a validated batch.
#[derive(Clone, Debug)]
pub struct FrameIter<'a> {
    rest: &'a [u8],
    next: u16,
    count: u16,
}

impl<'a> Iterator for FrameIter<'a> {
    type Item = Frame<'a>;

    fn next(&mut self) -> Option<Frame<'a>> {
        if self.next == self.count {
            return None;
        }
        // Validated by `frame`: the prefix and body are present.
        let (len, rest) = self.rest.split_first_chunk::<2>()?;
        let len = u16::from_be_bytes(*len) as usize;
        let (bytes, rest) = rest.split_at_checked(len)?;
        self.rest = rest;
        let frame = Frame {
            index: self.next,
            bytes,
        };
        self.next += 1;
        Some(frame)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = (self.count - self.next) as usize;
        (n, Some(n))
    }
}

impl ExactSizeIterator for FrameIter<'_> {}

/// A framed binary message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message<'a> {
    /// The one-byte heartbeat.
    Heartbeat,
    /// A validated batch of packets.
    Packets(Frames<'a>),
}

/// Frame one binary message.
pub fn frame(payload: &[u8], limits: FramingLimits) -> Result<Message<'_>, FramingError> {
    let err = |kind, offset, packet_index| FramingError {
        kind,
        offset,
        packet_index,
    };
    if payload.len() > limits.max_payload {
        return Err(err(
            DecodeDiagnosticKind::Oversized,
            limits.max_payload,
            None,
        ));
    }
    if payload.len() == 1 {
        return Ok(Message::Heartbeat);
    }
    let Some((count, body)) = payload.split_first_chunk::<2>() else {
        return Err(err(DecodeDiagnosticKind::Truncated, payload.len(), None));
    };
    let count = u16::from_be_bytes(*count);
    // Each packet needs at least its prefix and one byte.
    if count > limits.max_packets || count as usize * 3 > body.len() {
        return Err(err(DecodeDiagnosticKind::CountMismatch, 0, None));
    }
    let mut rest = body;
    for index in 0..count {
        let offset = payload.len() - rest.len();
        let Some((len, after)) = rest.split_first_chunk::<2>() else {
            return Err(err(DecodeDiagnosticKind::Truncated, offset, Some(index)));
        };
        let len = u16::from_be_bytes(*len) as usize;
        if len == 0 {
            return Err(err(
                DecodeDiagnosticKind::UnknownLength,
                offset,
                Some(index),
            ));
        }
        let Some((_, after)) = after.split_at_checked(len) else {
            return Err(err(
                DecodeDiagnosticKind::Truncated,
                payload.len(),
                Some(index),
            ));
        };
        rest = after;
    }
    if !rest.is_empty() {
        return Err(err(
            DecodeDiagnosticKind::TrailingBytes,
            payload.len() - rest.len(),
            None,
        ));
    }
    Ok(Message::Packets(Frames { body, count }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(packets: &[&[u8]]) -> Vec<u8> {
        let mut out = (packets.len() as u16).to_be_bytes().to_vec();
        for p in packets {
            out.extend((p.len() as u16).to_be_bytes());
            out.extend(*p);
        }
        out
    }

    fn kind(payload: &[u8]) -> DecodeDiagnosticKind {
        frame(payload, FramingLimits::default()).unwrap_err().kind()
    }

    #[test]
    fn edge_inputs_are_explicit_errors() {
        use DecodeDiagnosticKind::*;
        assert_eq!(kind(&[]), Truncated);
        assert_eq!(
            frame(&[7], FramingLimits::default()),
            Ok(Message::Heartbeat)
        );
        // A declared count the buffer cannot hold.
        assert_eq!(kind(&[0, 3, 0, 1, 9]), CountMismatch);
        // A trailing partial packet.
        let mut b = batch(&[&[1; 8]]);
        b.extend([0, 8, 1]);
        assert_eq!(kind(&b), TrailingBytes);
        // A zero-length packet.
        assert_eq!(kind(&[0, 1, 0, 0, 0]), UnknownLength);
        // An empty batch is valid.
        match frame(&[0, 0], FramingLimits::default()).unwrap() {
            Message::Packets(f) => assert!(f.is_empty()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_10_mib_input_is_refused_before_any_work() {
        let big = vec![0xFF; 10 << 20];
        let e = frame(&big, FramingLimits::default()).unwrap_err();
        assert_eq!(e.kind(), DecodeDiagnosticKind::Oversized);
        let limits = FramingLimits::default().with_max_payload(16 << 20).unwrap();
        // Within the payload bound, the count is still checked first.
        assert_eq!(
            frame(&big, limits).unwrap_err().kind(),
            DecodeDiagnosticKind::CountMismatch
        );
    }

    #[test]
    fn the_packet_bound_is_checked_before_iteration() {
        let packets: Vec<Vec<u8>> = (0..5).map(|_| vec![1; 8]).collect();
        let refs: Vec<&[u8]> = packets.iter().map(Vec::as_slice).collect();
        let b = batch(&refs);
        let four = FramingLimits::default().with_max_packets(4).unwrap();
        assert_eq!(
            frame(&b, four).unwrap_err().kind(),
            DecodeDiagnosticKind::CountMismatch
        );
        let five = FramingLimits::default().with_max_packets(5).unwrap();
        let Message::Packets(f) = frame(&b, five).unwrap() else {
            panic!()
        };
        assert_eq!(f.iter().len(), 5);
    }

    #[test]
    fn frames_borrow_the_input_in_order() {
        let b = batch(&[&[1; 8], &[2; 184], &[3; 32], &[4; 16]]);
        let Message::Packets(f) = frame(&b, FramingLimits::default()).unwrap() else {
            panic!()
        };
        let families: Vec<_> = f.iter().map(|x| (x.index(), x.family())).collect();
        assert_eq!(
            families,
            [
                (0, PacketFamily::Ltp),
                (1, PacketFamily::Full),
                (2, PacketFamily::IndexFull),
                (3, PacketFamily::Unknown)
            ]
        );
        assert!(f.iter().nth(1).unwrap().bytes().iter().all(|b| *b == 2));
    }

    #[test]
    fn bounds_are_validated() {
        let d = FramingLimits::default();
        assert!(d.with_max_payload((64 << 10) - 1).is_err());
        assert!(d.with_max_payload((16 << 20) + 1).is_err());
        assert!(d.with_max_packets(0).is_err());
        assert!(d.with_max_packets(u16::MAX).is_ok());
    }
}
