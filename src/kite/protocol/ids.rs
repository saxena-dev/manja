//! Checked broker identifiers.
//!
use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Kite instrument token: an unsigned 32-bit numeric identifier
/// (`kite:postbacks.md:71`,
/// `kite:websocket.md:93`).
///
/// A token identifies an instrument only on the day it is used: exchanges may
/// reuse tokens for different derivative instruments after expiry
/// (`kite:market-quotes.md:56`), so a token is not a stable
/// historical identity, and this type claims no ownership of an instrument
/// registry. A token alone establishes neither tradability nor the exchange
/// segment.
///
/// It deserializes from a JSON number or a string of decimal digits, because
/// both forms occur in broker responses, and rejects anything outside
/// `0..=u32::MAX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstrumentToken(u32);

impl InstrumentToken {
    /// Wrap a raw token.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The raw token.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for InstrumentToken {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<InstrumentToken> for u32 {
    fn from(value: InstrumentToken) -> Self {
        value.0
    }
}

impl TryFrom<i64> for InstrumentToken {
    type Error = std::num::TryFromIntError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        u32::try_from(value).map(Self)
    }
}

impl std::str::FromStr for InstrumentToken {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u32>().map(Self)
    }
}

impl fmt::Display for InstrumentToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for InstrumentToken {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(self.0)
    }
}

impl<'de> Deserialize<'de> for InstrumentToken {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct TokenVisitor;

        impl Visitor<'_> for TokenVisitor {
            type Value = InstrumentToken;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an instrument token (u32, as a number or decimal string)")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                u32::try_from(v)
                    .map(InstrumentToken)
                    .map_err(|_| E::custom("instrument token out of u32 range"))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                InstrumentToken::try_from(v)
                    .map_err(|_| E::custom("instrument token out of u32 range"))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse()
                    .map_err(|_| E::custom("instrument token string is not a u32"))
            }
        }

        d.deserialize_any(TokenVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_numbers_and_digit_strings_in_range() {
        let t: InstrumentToken = serde_json::from_str("408065").unwrap();
        assert_eq!(t.get(), 408065);
        let t: InstrumentToken = serde_json::from_str("\"779521\"").unwrap();
        assert_eq!(t.get(), 779521);
        assert_eq!(serde_json::to_string(&t).unwrap(), "779521");
    }

    #[test]
    fn rejects_out_of_range_and_non_numeric_values() {
        for bad in ["-1", "4294967296", "\"12a\"", "1.5", "null"] {
            assert!(
                serde_json::from_str::<InstrumentToken>(bad).is_err(),
                "{bad}"
            );
        }
        assert!(InstrumentToken::try_from(-5i64).is_err());
    }
}
