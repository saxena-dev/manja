//! `Exchange` enum.
//!
//! [`Exchange`] names the exchange venues and segments used in Kite requests
//! and responses (`kite:orders.md:88`). Its wire
//! strings are exact and case-sensitive.
//!
//! Parsing is strict: an unrecognized string is an error from
//! [`str::parse`] and from `Deserialize`, never a silent fallback. Response
//! DTOs that must survive new venues wrap the value in
//! [`Inbound`](crate::kite::protocol::Inbound).
//!
//! Price scaling is not a property of this type; see
//! [`crate::kite::protocol::scale`], which also explains why the segment is
//! never derived from an instrument token.
//!
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::kite::protocol::{UnknownValue, WireEnum};

/// The exchanges venues supported by Kite Connect API.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Exchange {
    /// No exchange: the empty string. Some calculation responses omit the
    /// exchange; it is never a valid request value.
    #[default]
    NONE,
    /// National Stock Exchange
    NSE,
    /// National Futures and Options
    NFO,
    /// Currency Derivatives Segment
    CDS,
    /// Bombay Stock Exchange
    BSE,
    /// Bombay Futures and Options
    BFO,
    /// Bombay Currency Derivatives
    BCD,
    /// Multi Commodity Exchange
    MCX,
    /// Multi Commodity Exchange Stock Exchange
    MCXSX,
    /// Stock Market Indices
    INDICES,
}

impl WireEnum for Exchange {
    fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "" => Self::NONE,
            "NSE" => Self::NSE,
            "NFO" => Self::NFO,
            "CDS" => Self::CDS,
            "BSE" => Self::BSE,
            "BFO" => Self::BFO,
            "BCD" => Self::BCD,
            "MCX" => Self::MCX,
            "MCXSX" => Self::MCXSX,
            "INDICES" => Self::INDICES,
            _ => return None,
        })
    }

    fn as_wire(&self) -> &'static str {
        match self {
            Self::NONE => "",
            Self::NSE => "NSE",
            Self::NFO => "NFO",
            Self::CDS => "CDS",
            Self::BSE => "BSE",
            Self::BFO => "BFO",
            Self::BCD => "BCD",
            Self::MCX => "MCX",
            Self::MCXSX => "MCXSX",
            Self::INDICES => "INDICES",
        }
    }
}

impl Exchange {
    /// Whether instruments of this exchange can be ordered: every venue
    /// except `INDICES` and `NONE`.
    pub(crate) fn is_tradable(&self) -> bool {
        !matches!(self, Self::NONE | Self::INDICES)
    }
}

/// An unrecognized exchange string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownExchange(pub UnknownValue);

impl fmt::Display for UnknownExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown exchange {:?}", self.0.raw())
    }
}

impl std::error::Error for UnknownExchange {}

impl FromStr for Exchange {
    type Err = UnknownExchange;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_wire(s).ok_or_else(|| UnknownExchange(UnknownValue::new(s)))
    }
}

impl From<Exchange> for &str {
    fn from(value: Exchange) -> Self {
        value.as_wire()
    }
}

impl fmt::Display for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl Serialize for Exchange {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_wire())
    }
}

impl<'de> Deserialize<'de> for Exchange {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl TryFrom<crate::kite::protocol::Inbound<Exchange>> for Exchange {
    type Error = UnknownValue;

    fn try_from(v: crate::kite::protocol::Inbound<Exchange>) -> Result<Self, Self::Error> {
        match v {
            crate::kite::protocol::Inbound::Known(v) => Ok(v),
            crate::kite::protocol::Inbound::Unknown(u) => Err(u),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kite::protocol::Inbound;

    #[test]
    fn parsing_is_strict_and_exact() {
        assert_eq!("NSE".parse::<Exchange>(), Ok(Exchange::NSE));
        assert_eq!("".parse::<Exchange>(), Ok(Exchange::NONE));
        let err = "nse".parse::<Exchange>().unwrap_err();
        assert_eq!(err.0.raw(), "nse");
        assert!(serde_json::from_str::<Exchange>("\"MF\"").is_err());
    }

    #[test]
    fn inbound_preserves_new_venues() {
        let e: Inbound<Exchange> = serde_json::from_str("\"MF\"").unwrap();
        assert_eq!(e.as_wire(), "MF");
        assert!(Exchange::try_from(e).is_err());
    }

    #[test]
    fn tradability_excludes_indices() {
        assert!(Exchange::NFO.is_tradable());
        assert!(!Exchange::INDICES.is_tradable());
        assert!(!Exchange::NONE.is_tradable());
    }
}
