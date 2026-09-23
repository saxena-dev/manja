//! Binary packet families, field by field
//! (`kite:websocket.md:87-163`).
//!
//! Every field is read with explicit big-endian byte order at its
//! documented offset; nothing is cast from a native struct. Values stay as
//! the raw integers on the wire:
//!
//! - prices are raw `int32` in the segment's minor unit; turn one into a
//!   [`ScaledPrice`] with [`scaled`] and a caller-supplied [`Segment`]
//!   (currencies divide by 10 000 000, everything else by 100,
//!   `kite:websocket.md:89`; the BSE currency segment has no verified scale and
//!   is refused);
//! - quantities and order counts are unsigned. The wire type is signed, so
//!   a negative value is not a quantity: it is refused with
//!   [`PacketErrorKind::NegativeQuantity`] rather than reinterpreted;
//! - timestamps are the raw Unix seconds the exchange sent.
//!
//! Index packets (`kite:websocket.md:111-125`) are their own types, never padded
//! tradable quotes. Full packets carry exactly five bids and five offers;
//! each depth entry is `int32` quantity, `int32` price and `int16` orders
//! followed by two padding bytes (`kite:websocket.md:129`). The commented byte
//! table after that paragraph (`kite:websocket.md:131-163`) contradicts the prose
//! and the 184-byte total, and is not followed.
//!
//! An LTP packet carries a token and a price only: it establishes neither
//! tradability nor which instrument or segment the token denotes.
//!
//! # Differences from the reference outputs
//!
//! The official `ticker_*.json` expectations are already scaled floats from
//! a client library. Raw-integer expectations are therefore asserted
//! against the documented layout, and scaled values are compared
//! separately. Their tradable-quote `change` has no field in the tradable
//! packet and is not last price minus close, so it cannot be derived from
//! the packet and is not reproduced.
//!
use std::fmt;

use crate::kite::decoder::framing::{Frame, PacketFamily};
use crate::kite::protocol::scale::{ScaleError, Segment};
use crate::kite::protocol::{InstrumentToken, ScaledPrice};

/// The price of a raw wire integer in `segment`.
pub fn scaled(raw: i32, segment: Segment) -> Result<ScaledPrice, ScaleError> {
    ScaledPrice::from_raw(raw as i64, segment)
}

/// An LTP packet (8 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ltp {
    /// Instrument token.
    pub instrument_token: InstrumentToken,
    /// Last traded price, raw.
    pub last_price: i32,
}

/// The fields a quote and a full packet share (bytes 0-44).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuoteFields {
    /// Instrument token.
    pub instrument_token: InstrumentToken,
    /// Last traded price, raw.
    pub last_price: i32,
    /// Last traded quantity.
    pub last_quantity: u32,
    /// Average traded price, raw.
    pub average_price: i32,
    /// Volume traded for the day.
    pub volume: u32,
    /// Total buy quantity.
    pub buy_quantity: u32,
    /// Total sell quantity.
    pub sell_quantity: u32,
    /// Open price of the day, raw.
    pub open: i32,
    /// High price of the day, raw.
    pub high: i32,
    /// Low price of the day, raw.
    pub low: i32,
    /// Close price, raw.
    pub close: i32,
}

/// A tradable quote packet (44 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quote {
    /// The quote fields.
    pub fields: QuoteFields,
}

/// One market-depth entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DepthEntry {
    /// Quantity.
    pub quantity: u32,
    /// Price, raw.
    pub price: i32,
    /// Number of orders.
    pub orders: u16,
}

/// Five bids and five offers, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Depth {
    /// Bids.
    pub bids: [DepthEntry; 5],
    /// Offers.
    pub offers: [DepthEntry; 5],
}

/// A tradable full packet (184 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full {
    /// The quote fields.
    pub fields: QuoteFields,
    /// Last traded time, raw Unix seconds.
    pub last_trade_time: u32,
    /// Open interest.
    pub open_interest: u32,
    /// Open interest day high.
    pub open_interest_day_high: u32,
    /// Open interest day low.
    pub open_interest_day_low: u32,
    /// Exchange timestamp, raw Unix seconds.
    pub exchange_timestamp: u32,
    /// Market depth.
    pub depth: Depth,
}

/// The fields of an index packet (bytes 0-28).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexFields {
    /// Index token.
    pub instrument_token: InstrumentToken,
    /// Last traded price, raw.
    pub last_price: i32,
    /// High of the day, raw.
    pub high: i32,
    /// Low of the day, raw.
    pub low: i32,
    /// Open of the day, raw.
    pub open: i32,
    /// Close of the day, raw.
    pub close: i32,
    /// Price change, raw; may be negative.
    pub change: i32,
}

/// An index quote packet (28 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexQuote {
    /// The index fields.
    pub fields: IndexFields,
}

/// An index full packet (32 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexFull {
    /// The index fields.
    pub fields: IndexFields,
    /// Exchange timestamp, raw Unix seconds.
    pub exchange_timestamp: u32,
}

/// A decoded packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Packet {
    /// LTP.
    Ltp(Ltp),
    /// Tradable quote.
    Quote(Quote),
    /// Tradable full, with depth.
    Full(Full),
    /// Index quote.
    IndexQuote(IndexQuote),
    /// Index full.
    IndexFull(IndexFull),
}

impl Packet {
    /// The instrument token.
    pub fn instrument_token(&self) -> InstrumentToken {
        match self {
            Self::Ltp(p) => p.instrument_token,
            Self::Quote(p) => p.fields.instrument_token,
            Self::Full(p) => p.fields.instrument_token,
            Self::IndexQuote(p) => p.fields.instrument_token,
            Self::IndexFull(p) => p.fields.instrument_token,
        }
    }
}

/// Why a packet could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PacketErrorKind {
    /// No documented family has this length.
    UnknownLength(usize),
    /// A quantity, order count or timestamp field was negative on the wire.
    NegativeQuantity {
        /// Byte offset of the field.
        offset: usize,
    },
}

/// A packet that could not be decoded, with its position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketError {
    /// Position within the message.
    pub packet_index: u16,
    /// What went wrong.
    pub kind: PacketErrorKind,
}

impl fmt::Display for PacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            PacketErrorKind::UnknownLength(n) => {
                write!(
                    f,
                    "packet {}: no documented family is {n} bytes",
                    self.packet_index
                )
            }
            PacketErrorKind::NegativeQuantity { offset } => write!(
                f,
                "packet {}: the unsigned field at byte {offset} is negative",
                self.packet_index
            ),
        }
    }
}

impl std::error::Error for PacketError {}

// Big-endian readers over a packet whose length is already known.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn i32(&self, at: usize) -> i32 {
        i32::from_be_bytes([self.0[at], self.0[at + 1], self.0[at + 2], self.0[at + 3]])
    }

    fn u32(&self, at: usize) -> Result<u32, PacketErrorKind> {
        u32::try_from(self.i32(at)).map_err(|_| PacketErrorKind::NegativeQuantity { offset: at })
    }

    fn u16(&self, at: usize) -> Result<u16, PacketErrorKind> {
        let v = i16::from_be_bytes([self.0[at], self.0[at + 1]]);
        u16::try_from(v).map_err(|_| PacketErrorKind::NegativeQuantity { offset: at })
    }

    fn token(&self) -> InstrumentToken {
        InstrumentToken::new(u32::from_be_bytes([
            self.0[0], self.0[1], self.0[2], self.0[3],
        ]))
    }

    fn quote(&self) -> Result<QuoteFields, PacketErrorKind> {
        Ok(QuoteFields {
            instrument_token: self.token(),
            last_price: self.i32(4),
            last_quantity: self.u32(8)?,
            average_price: self.i32(12),
            volume: self.u32(16)?,
            buy_quantity: self.u32(20)?,
            sell_quantity: self.u32(24)?,
            open: self.i32(28),
            high: self.i32(32),
            low: self.i32(36),
            close: self.i32(40),
        })
    }

    fn index(&self) -> IndexFields {
        IndexFields {
            instrument_token: self.token(),
            last_price: self.i32(4),
            high: self.i32(8),
            low: self.i32(12),
            open: self.i32(16),
            close: self.i32(20),
            change: self.i32(24),
        }
    }

    fn depth_entry(&self, at: usize) -> Result<DepthEntry, PacketErrorKind> {
        // Bytes at+10..at+12 are padding.
        Ok(DepthEntry {
            quantity: self.u32(at)?,
            price: self.i32(at + 4),
            orders: self.u16(at + 8)?,
        })
    }

    fn depth(&self) -> Result<Depth, PacketErrorKind> {
        let mut bids = [DepthEntry::default(); 5];
        let mut offers = [DepthEntry::default(); 5];
        for i in 0..5 {
            bids[i] = self.depth_entry(64 + 12 * i)?;
            offers[i] = self.depth_entry(124 + 12 * i)?;
        }
        Ok(Depth { bids, offers })
    }
}

/// Decode one framed packet.
pub fn decode(frame: &Frame<'_>) -> Result<Packet, PacketError> {
    decode_bytes(frame.bytes()).map_err(|kind| PacketError {
        packet_index: frame.index(),
        kind,
    })
}

/// Decode one bare packet, such as the official `ticker_*.packet` samples.
pub fn decode_bytes(bytes: &[u8]) -> Result<Packet, PacketErrorKind> {
    let r = Reader(bytes);
    Ok(match PacketFamily::of_len(bytes.len()) {
        PacketFamily::Ltp => Packet::Ltp(Ltp {
            instrument_token: r.token(),
            last_price: r.i32(4),
        }),
        PacketFamily::Quote => Packet::Quote(Quote { fields: r.quote()? }),
        PacketFamily::Full => Packet::Full(Full {
            fields: r.quote()?,
            last_trade_time: r.u32(44)?,
            open_interest: r.u32(48)?,
            open_interest_day_high: r.u32(52)?,
            open_interest_day_low: r.u32(56)?,
            exchange_timestamp: r.u32(60)?,
            depth: r.depth()?,
        }),
        PacketFamily::IndexQuote => Packet::IndexQuote(IndexQuote { fields: r.index() }),
        PacketFamily::IndexFull => Packet::IndexFull(IndexFull {
            fields: r.index(),
            exchange_timestamp: r.u32(28)?,
        }),
        _ => return Err(PacketErrorKind::UnknownLength(bytes.len())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_with(offset: usize, value: [u8; 4]) -> Vec<u8> {
        let mut p = vec![0; 184];
        p[offset..offset + 4].copy_from_slice(&value);
        p
    }

    #[test]
    fn negative_unsigned_fields_are_refused_not_reinterpreted() {
        for offset in [8, 16, 20, 24, 44, 48, 60, 64, 124 + 48] {
            assert_eq!(
                decode_bytes(&full_with(offset, (-1i32).to_be_bytes())),
                Err(PacketErrorKind::NegativeQuantity { offset }),
                "{offset}"
            );
        }
        // Depth orders are int16.
        let mut p = vec![0; 184];
        p[72..74].copy_from_slice(&(-2i16).to_be_bytes());
        assert_eq!(
            decode_bytes(&p),
            Err(PacketErrorKind::NegativeQuantity { offset: 72 })
        );
        // Padding is ignored.
        let mut p = vec![0; 184];
        p[74..76].copy_from_slice(&[0xFF, 0xFF]);
        assert!(decode_bytes(&p).is_ok());
    }

    #[test]
    fn prices_keep_their_sign_and_scale_by_segment() {
        let mut p = vec![0; 28];
        p[24..28].copy_from_slice(&(-2917i32).to_be_bytes());
        let Ok(Packet::IndexQuote(q)) = decode_bytes(&p) else {
            panic!()
        };
        assert_eq!(q.fields.change, -2917);
        assert_eq!(
            scaled(i32::MAX, Segment::Nse).unwrap().raw(),
            i32::MAX as i64
        );
        assert_eq!(
            scaled(i32::MIN, Segment::Cds).unwrap().divisor(),
            10_000_000
        );
        assert!(scaled(100, Segment::Bcd).is_err(), "no verified BCD scale");
    }

    #[test]
    fn unknown_lengths_are_explicit() {
        for len in [0, 1, 16, 45, 183, 185] {
            assert_eq!(
                decode_bytes(&vec![0; len]),
                Err(PacketErrorKind::UnknownLength(len))
            );
        }
    }
}
