//! Per-segment price scale policy for integer prices from the ticker.
//!
//! Binary ticker packets carry prices as `int32`. The protocol reference
//! states: "For currencies, the `int32` price values should be divided by
//! 10000000 ... For everything else, the price values should be divided by
//! 100" (`kite:websocket.md:89`). That sentence also
//! says the currency divisor yields "four decimal places", which a divisor of
//! 10 000 000 does not; the explicit divisor is what this policy implements.
//!
//! | Segment | Divisor | Status and evidence |
//! |---|---|---|
//! | CDS | 10 000 000 | supported: `kite:websocket.md:89` "currencies" (`docs/contract.md` §5) |
//! | BCD | — | **disabled**: the reference names no BCD rule; [`ScaleError::UnsupportedSegment`] |
//! | NSE, NFO, BSE, BFO, MCX, MCXSX, INDICES | 100 | supported: `kite:websocket.md:89` "everything else" |
//!
//! The previous helper's CDS 1 000 000 and BCD 1 000 values had no protocol
//! evidence and are not used. Raw integers stay available for every segment,
//! including disabled ones.
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
            Exchange::MCX => Self::Mcx,
            Exchange::MCXSX => Self::McxSx,
            Exchange::INDICES => Self::Indices,
            Exchange::NONE => return None,
        })
    }
}

/// A segment whose scale contract is not verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScaleError {
    /// No verified divisor exists for this segment; the raw integer is still
    /// available, but no scaled price is produced.
    UnsupportedSegment(Segment),
}

impl fmt::Display for ScaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSegment(s) => write!(f, "price scale for {s:?} is not supported"),
        }
    }
}

impl std::error::Error for ScaleError {}

/// The divisor that turns a ticker `int32` price of `segment` into rupees.
pub fn price_divisor(segment: Segment) -> Result<u32, ScaleError> {
    match segment {
        Segment::Cds => Ok(10_000_000),
        Segment::Bcd => Err(ScaleError::UnsupportedSegment(segment)),
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
        // Golden: an NSE raw price of 157 610 paise is 1576.10 rupees
        // (the vendored single_full case, raw last_traded_price 157610).
        let p = ScaledPrice::from_raw(157_610, Segment::Nse).unwrap();
        assert_eq!((p.raw(), p.divisor()), (157_610, 100));
        assert_eq!(p.to_f64(), 1576.10);
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
    fn bcd_is_disabled_with_an_explicit_error() {
        assert_eq!(
            price_divisor(Segment::Bcd),
            Err(ScaleError::UnsupportedSegment(Segment::Bcd))
        );
        assert!(ScaledPrice::from_raw(1_000, Segment::Bcd).is_err());
    }

    #[test]
    fn segment_comes_from_an_exchange_value_not_a_token() {
        assert_eq!(Segment::from_exchange(&Exchange::CDS), Some(Segment::Cds));
        assert_eq!(Segment::from_exchange(&Exchange::NONE), None);
    }
}
