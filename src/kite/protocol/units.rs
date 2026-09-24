//! Exact price and quantity units with checked conversion.
//!
//! A [`ScaledPrice`] is an exact rational `raw / divisor`: the raw broker
//! integer and its scale are always both available, and no value passes
//! silently through `f64`. Conversions follow the policy versioned as
//! [`CONVERSION_POLICY_VERSION`], independently of any storage format:
//!
//! 1. `raw → f64` ([`ScaledPrice::to_f64`]) is one IEEE-754 division, which
//!    rounds to the nearest representable `f64` (ties to even). It is for
//!    display and legacy interfaces; exact arithmetic uses `raw` and
//!    `divisor`.
//! 2. `f64 → raw` ([`ScaledPrice::from_f64`]) rejects NaN and infinities,
//!    multiplies by the divisor, and accepts the result only if it lies
//!    within `1e-6` of an integer and inside the `i64` range; the nearest
//!    integer is then the raw value. Anything else is
//!    [`UnitError::Unrepresentable`] rather than a rounded guess.
//! 3. Arithmetic ([`ScaledPrice::checked_mul`]) is checked and fails with
//!    [`UnitError::Overflow`].
//!
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::kite::protocol::scale::{ScaleError, Segment, price_divisor};

/// Version of the numeric conversion policy described in this module.
///
/// Incremented whenever a conversion rule changes its result for any input,
/// so derived values can record which policy produced them.
pub const CONVERSION_POLICY_VERSION: u32 = 1;

/// Largest distance from an integer, in raw units, that `from_f64` accepts as
/// float representation noise.
const FLOAT_TOLERANCE: f64 = 1e-6;

/// Why a unit conversion failed.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum UnitError {
    /// The input float was NaN or infinite.
    NonFinite,
    /// The input is not an exact multiple of the scale's unit, or is out of
    /// range.
    Unrepresentable {
        /// The rejected value.
        value: f64,
        /// The scale divisor it was checked against.
        divisor: u32,
    },
    /// A divisor of zero was supplied.
    ZeroDivisor,
    /// Checked arithmetic overflowed.
    Overflow,
    /// A quantity of zero was supplied where a positive quantity is required.
    ZeroQuantity,
    /// The scale of the segment is not supported.
    Scale(ScaleError),
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => f.write_str("value is not finite"),
            Self::Unrepresentable { value, divisor } => {
                write!(f, "{value} is not an exact multiple of 1/{divisor}")
            }
            Self::ZeroDivisor => f.write_str("divisor is zero"),
            Self::Overflow => f.write_str("arithmetic overflow"),
            Self::ZeroQuantity => f.write_str("quantity must be positive"),
            Self::Scale(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for UnitError {}

impl From<ScaleError> for UnitError {
    fn from(e: ScaleError) -> Self {
        Self::Scale(e)
    }
}

/// An exact price, `raw / divisor` rupees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScaledPrice {
    raw: i64,
    divisor: u32,
}

impl ScaledPrice {
    /// Build from a raw integer and an explicit divisor.
    pub fn new(raw: i64, divisor: u32) -> Result<Self, UnitError> {
        if divisor == 0 {
            return Err(UnitError::ZeroDivisor);
        }
        Ok(Self { raw, divisor })
    }

    /// Build from a raw ticker integer of `segment`, using the verified
    /// divisor of [`price_divisor`]. Unsupported segments fail explicitly.
    pub fn from_raw(raw: i64, segment: Segment) -> Result<Self, ScaleError> {
        Ok(Self {
            raw,
            divisor: price_divisor(segment)?,
        })
    }

    /// Convert a legacy float price exactly, under policy rule 2.
    pub fn from_f64(value: f64, divisor: u32) -> Result<Self, UnitError> {
        if divisor == 0 {
            return Err(UnitError::ZeroDivisor);
        }
        if !value.is_finite() {
            return Err(UnitError::NonFinite);
        }
        let scaled = value * divisor as f64;
        let nearest = scaled.round();
        let in_range = nearest >= i64::MIN as f64 && nearest < i64::MAX as f64;
        if !in_range || (scaled - nearest).abs() > FLOAT_TOLERANCE {
            return Err(UnitError::Unrepresentable { value, divisor });
        }
        Ok(Self {
            raw: nearest as i64,
            divisor,
        })
    }

    /// The raw integer.
    pub fn raw(self) -> i64 {
        self.raw
    }

    /// The divisor.
    pub fn divisor(self) -> u32 {
        self.divisor
    }

    /// Nearest `f64`, under policy rule 1.
    pub fn to_f64(self) -> f64 {
        self.raw as f64 / self.divisor as f64
    }

    /// Multiply by a signed quantity, for example to compute a notional
    /// value, keeping the divisor.
    pub fn checked_mul(self, quantity: i64) -> Result<Self, UnitError> {
        let raw = self.raw.checked_mul(quantity).ok_or(UnitError::Overflow)?;
        Ok(Self { raw, ..self })
    }
}

/// A positive order quantity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct Quantity(u32);

impl Quantity {
    /// Wrap a positive quantity; zero is rejected.
    pub fn new(value: u32) -> Result<Self, UnitError> {
        if value == 0 {
            Err(UnitError::ZeroQuantity)
        } else {
            Ok(Self(value))
        }
    }

    /// The quantity.
    pub fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for Quantity {
    type Error = UnitError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Quantity> for u32 {
    fn from(q: Quantity) -> Self {
        q.0
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_round_trips() {
        for (value, divisor, raw) in [
            (1576.1, 100, 157_610),
            (0.05, 100, 5),
            (-12.34, 100, -1_234),
            (83.1234, 10_000_000, 831_234_000),
            (0.0, 100, 0),
        ] {
            let p = ScaledPrice::from_f64(value, divisor).unwrap();
            assert_eq!(p.raw(), raw, "{value}");
            assert_eq!(p.to_f64(), value);
        }
    }

    #[test]
    fn non_finite_and_unrepresentable_floats_are_rejected() {
        assert_eq!(
            ScaledPrice::from_f64(f64::NAN, 100),
            Err(UnitError::NonFinite)
        );
        assert_eq!(
            ScaledPrice::from_f64(f64::INFINITY, 100),
            Err(UnitError::NonFinite)
        );
        // Not a multiple of a paisa: rejected, not rounded.
        assert!(matches!(
            ScaledPrice::from_f64(1.005, 100),
            Err(UnitError::Unrepresentable { .. })
        ));
        assert!(matches!(
            ScaledPrice::from_f64(1e300, 100),
            Err(UnitError::Unrepresentable { .. })
        ));
        assert_eq!(ScaledPrice::from_f64(1.0, 0), Err(UnitError::ZeroDivisor));
    }

    #[test]
    fn arithmetic_is_checked() {
        let p = ScaledPrice::new(i64::MAX / 2, 100).unwrap();
        assert_eq!(p.checked_mul(3), Err(UnitError::Overflow));
        let p = ScaledPrice::new(157_610, 100).unwrap();
        assert_eq!(p.checked_mul(-3).unwrap().raw(), -472_830);
    }

    #[test]
    fn quantities_are_positive() {
        assert_eq!(Quantity::new(0), Err(UnitError::ZeroQuantity));
        assert_eq!(Quantity::new(5).unwrap().get(), 5);
        assert!(serde_json::from_str::<Quantity>("0").is_err());
        assert!(serde_json::from_str::<Quantity>("-1").is_err());
        assert_eq!(
            serde_json::to_string(&Quantity::new(7).unwrap()).unwrap(),
            "7"
        );
    }
}
