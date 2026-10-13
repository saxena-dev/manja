//! Binary packet families, field by field
//! (`kite:websocket.md:87-163`).
//!
//! Every field is read with explicit big-endian byte order at its
//! documented offset; nothing is cast from a native struct. Values stay as
//! the raw integers on the wire:
//!
//! - prices are raw `int32` in the segment's minor unit; turn one into a
//!   [`ScaledPrice`] with [`scaled`] and a caller-supplied [`Segment`]
//!   (currencies divide by 10 000 000 and everything else by 100,
//!   `kite:websocket.md:89`, except BSE currency and NSE commodity
//!   derivatives, which divide by 10 000 as Zerodha's client library does);
//! - quantities, volume, open interest and order counts are read unsigned:
//!   `u32`, and `u16` for depth order counts. Kite's page types them
//!   `int32` and `int16`, but the server encodes them unsigned: a futures
//!   contract's open interest above `i32::MAX` was observed on the wire, and
//!   Zerodha's own client library reads every one of these fields unsigned
//!   (`docs/contract.md` §5). A count is never negative, so the high bit can
//!   only be magnitude, and no value of these fields is refused;
//! - timestamps are the raw Unix seconds the exchange sent, read unsigned.
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

    // Counts and timestamps: unsigned on the wire, never refused.
    fn u32(&self, at: usize) -> u32 {
        u32::from_be_bytes([self.0[at], self.0[at + 1], self.0[at + 2], self.0[at + 3]])
    }

    fn u16(&self, at: usize) -> u16 {
        u16::from_be_bytes([self.0[at], self.0[at + 1]])
    }

    fn token(&self) -> InstrumentToken {
        InstrumentToken::new(u32::from_be_bytes([
            self.0[0], self.0[1], self.0[2], self.0[3],
        ]))
    }

    fn quote(&self) -> QuoteFields {
        QuoteFields {
            instrument_token: self.token(),
            last_price: self.i32(4),
            last_quantity: self.u32(8),
            average_price: self.i32(12),
            volume: self.u32(16),
            buy_quantity: self.u32(20),
            sell_quantity: self.u32(24),
            open: self.i32(28),
            high: self.i32(32),
            low: self.i32(36),
            close: self.i32(40),
        }
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

    fn depth_entry(&self, at: usize) -> DepthEntry {
        // Bytes at+10..at+12 are padding.
        DepthEntry {
            quantity: self.u32(at),
            price: self.i32(at + 4),
            orders: self.u16(at + 8),
        }
    }

    fn depth(&self) -> Depth {
        let mut bids = [DepthEntry::default(); 5];
        let mut offers = [DepthEntry::default(); 5];
        for i in 0..5 {
            bids[i] = self.depth_entry(64 + 12 * i);
            offers[i] = self.depth_entry(124 + 12 * i);
        }
        Depth { bids, offers }
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
        PacketFamily::Quote => Packet::Quote(Quote { fields: r.quote() }),
        PacketFamily::Full => Packet::Full(Full {
            fields: r.quote(),
            last_trade_time: r.u32(44),
            open_interest: r.u32(48),
            open_interest_day_high: r.u32(52),
            open_interest_day_low: r.u32(56),
            exchange_timestamp: r.u32(60),
            depth: r.depth(),
        }),
        PacketFamily::IndexQuote => Packet::IndexQuote(IndexQuote { fields: r.index() }),
        PacketFamily::IndexFull => Packet::IndexFull(IndexFull {
            fields: r.index(),
            exchange_timestamp: r.u32(28),
        }),
        _ => return Err(PacketErrorKind::UnknownLength(bytes.len())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_and_timestamp_fields_are_unsigned_on_the_wire() {
        // Open interest above i32::MAX, as observed for a futures contract
        // with a large lot (contract.md §5): read unsigned, never refused.
        let mut p = vec![0; 184];
        for at in [48, 52, 56] {
            p[at..at + 4].copy_from_slice(&3_000_000_000u32.to_be_bytes());
        }
        for at in [8, 16, 20, 24, 64, 124 + 48] {
            p[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        }
        // Timestamps past 2038 (the signed limit) stay positive seconds.
        p[44..48].copy_from_slice(&0x8000_0000u32.to_be_bytes());
        p[60..64].copy_from_slice(&0x8000_0001u32.to_be_bytes());
        // Depth orders above i16::MAX: the overflow Zerodha's own library
        // fixed in 2018.
        p[72..74].copy_from_slice(&40_000u16.to_be_bytes());
        let Ok(Packet::Full(f)) = decode_bytes(&p) else {
            panic!("a full packet")
        };
        assert_eq!(
            (
                f.open_interest,
                f.open_interest_day_high,
                f.open_interest_day_low
            ),
            (3_000_000_000, 3_000_000_000, 3_000_000_000)
        );
        assert_eq!(
            (
                f.fields.last_quantity,
                f.fields.volume,
                f.fields.buy_quantity,
                f.fields.sell_quantity
            ),
            (u32::MAX, u32::MAX, u32::MAX, u32::MAX)
        );
        assert_eq!(f.depth.bids[0].quantity, u32::MAX);
        assert_eq!(f.depth.offers[4].quantity, u32::MAX);
        assert_eq!(f.depth.bids[0].orders, 40_000);
        assert_eq!(
            (f.last_trade_time, f.exchange_timestamp),
            (0x8000_0000, 0x8000_0001)
        );
        let mut p = vec![0; 32];
        p[28..32].copy_from_slice(&u32::MAX.to_be_bytes());
        let Ok(Packet::IndexFull(i)) = decode_bytes(&p) else {
            panic!("an index full packet")
        };
        assert_eq!(i.exchange_timestamp, u32::MAX);
        // Padding is ignored.
        let mut p = vec![0; 184];
        p[74..76].copy_from_slice(&[0xFF, 0xFF]);
        assert!(decode_bytes(&p).is_ok());
        // Prices keep their sign: the same bit pattern is negative there.
        let mut p = vec![0; 184];
        p[4..8].copy_from_slice(&(-1i32).to_be_bytes());
        p[68..72].copy_from_slice(&(-5i32).to_be_bytes());
        let Ok(Packet::Full(f)) = decode_bytes(&p) else {
            panic!("a full packet")
        };
        assert_eq!((f.fields.last_price, f.depth.bids[0].price), (-1, -5));
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
        // BCD and NCO divide by 10 000, as Zerodha's own library does.
        assert_eq!(scaled(1_234_567, Segment::Bcd).unwrap().divisor(), 10_000);
        assert_eq!(scaled(1_234_567, Segment::Nco).unwrap().to_f64(), 123.4567);
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
