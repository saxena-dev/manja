//! Per-segment price scale policy for integer prices from the ticker.
//!
//! Binary ticker packets carry prices as `int32`. The protocol reference
//! states: "For currencies, the `int32` price values should be divided by
//! 10000000 ... For everything else, the price values should be divided by
//! 100" (`kite:websocket.md:89`). That sentence also
//! says the currency divisor yields "four decimal places", which a divisor of
//! 10 000 000 does not; the explicit divisor is what this policy implements.
//! The reference names neither BSE currency derivatives nor NSE commodity
//! derivatives; its general rules would give BCD 10 000 000 and NCO 100.
//! Zerodha's own client library, pykiteconnect 5.2.2, divides both by
//! 10 000, and this policy follows it (`docs/contract.md` §5).
//!
//! | Segment | Divisor | Evidence |
//! |---|---|---|
//! | CDS | 10 000 000 | `kite:websocket.md:89` "currencies" |
//! | BCD, NCO | 10 000 | pykiteconnect 5.2.2 (`docs/contract.md` §5) |
//! | NSE, NFO, BSE, BFO, MCX, MCXSX, INDICES | 100 | `kite:websocket.md:89` "everything else" |
//!
//! Every segment has a divisor, so no price is refused, and the raw integer
//! stays available as well. The previous helper's CDS 1 000 000 and BCD 1 000
//! values had no evidence and are not used.
//!
//! The protocol reference does not document how to derive the segment from an
//! instrument token, so this crate never infers it: the caller supplies the
//! [`Segment`], for example from the instrument master's `exchange`/`segment`
//! columns.
//!
use std::fmt;

use crate::kite::connect::models::Exchange;

/// A price-scale segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Segment {
    /// National Stock Exchange, equity.
    Nse,
    /// NSE futures and options.
    Nfo,
    /// NSE currency derivatives.
    Cds,
    /// Bombay Stock Exchange, equity.
    Bse,
    /// BSE futures and options.
    Bfo,
    /// BSE currency derivatives.
    Bcd,
    /// NSE commodity derivatives (instrument-master exchange `NCO`).
    Nco,
    /// Multi Commodity Exchange.
    Mcx,
    /// MCX Stock Exchange.
    McxSx,
    /// Indices.
    Indices,
}

impl Segment {
    /// The segment of an [`Exchange`] value, or `None` for `Exchange::NONE`.
    pub fn from_exchange(exchange: &Exchange) -> Option<Self> {
        Some(match exchange {
            Exchange::NSE => Self::Nse,
            Exchange::NFO => Self::Nfo,
            Exchange::CDS => Self::Cds,
            Exchange::BSE => Self::Bse,
            Exchange::BFO => Self::Bfo,
            Exchange::BCD => Self::Bcd,
            Exchange::NCO => Self::Nco,
            Exchange::MCX => Self::Mcx,
            Exchange::MCXSX => Self::McxSx,
            Exchange::INDICES => Self::Indices,
            Exchange::NONE => return None,
        })
    }
}

/// A segment whose price scale is not supported. Every segment has a
/// divisor since `CONVERSION_POLICY_VERSION` 2, so no value of this type is
/// produced; the type stays so that [`price_divisor`] keeps its signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScaleError {
    /// Never produced since `CONVERSION_POLICY_VERSION` 2. Kept so that code
    /// naming it still compiles.
    #[deprecated(
        since = "0.3.0",
        note = "never produced: every segment has a divisor (docs/contract.md §5)"
    )]
    UnsupportedSegment(Segment),
}

impl fmt::Display for ScaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[allow(deprecated)]
            Self::UnsupportedSegment(s) => write!(f, "price scale for {s:?} is not supported"),
        }
    }
}

impl std::error::Error for ScaleError {}

/// The divisor that turns a ticker `int32` price of `segment` into rupees.
pub fn price_divisor(segment: Segment) -> Result<u32, ScaleError> {
    match segment {
        Segment::Cds => Ok(10_000_000),
        Segment::Bcd | Segment::Nco => Ok(10_000),
        Segment::Nse
        | Segment::Nfo
        | Segment::Bse
        | Segment::Bfo
        | Segment::Mcx
        | Segment::McxSx
        | Segment::Indices => Ok(100),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kite::protocol::ScaledPrice;

    #[test]
    fn documented_divisors_and_golden_values() {
        assert_eq!(price_divisor(Segment::Nse), Ok(100));
        assert_eq!(price_divisor(Segment::Indices), Ok(100));
        assert_eq!(price_divisor(Segment::Cds), Ok(10_000_000));
        // Golden: an NSE raw price of 123 456 paise is 1234.56 rupees.
        let p = ScaledPrice::from_raw(123_456, Segment::Nse).unwrap();
        assert_eq!((p.raw(), p.divisor()), (123_456, 100));
        assert_eq!(p.to_f64(), 1234.56);
        // Golden: a CDS raw price of 831 234 000 is 83.1234 rupees.
        let p = ScaledPrice::from_raw(831_234_000, Segment::Cds).unwrap();
        assert_eq!(p.to_f64(), 83.1234);
        // Boundaries of the int32 wire type.
        let max = ScaledPrice::from_raw(i32::MAX as i64, Segment::Nse).unwrap();
        assert_eq!(max.to_f64(), 21_474_836.47);
        let min = ScaledPrice::from_raw(i32::MIN as i64, Segment::Nse).unwrap();
        assert_eq!(min.raw(), -2_147_483_648);
    }

    #[test]
    fn bcd_and_nco_divide_by_ten_thousand() {
        assert_eq!(price_divisor(Segment::Bcd), Ok(10_000));
        assert_eq!(price_divisor(Segment::Nco), Ok(10_000));
        // Golden: an NCO raw price of 7 512 345 is 751.2345 rupees.
        let p = ScaledPrice::from_raw(7_512_345, Segment::Nco).unwrap();
        assert_eq!(
            (p.raw(), p.divisor(), p.to_f64()),
            (7_512_345, 10_000, 751.2345)
        );
        assert_eq!(Segment::from_exchange(&Exchange::NCO), Some(Segment::Nco));
        assert_eq!(Segment::from_exchange(&Exchange::BCD), Some(Segment::Bcd));
    }

    #[test]
    fn segment_comes_from_an_exchange_value_not_a_token() {
        assert_eq!(Segment::from_exchange(&Exchange::CDS), Some(Segment::Cds));
        assert_eq!(Segment::from_exchange(&Exchange::NONE), None);
    }
}
