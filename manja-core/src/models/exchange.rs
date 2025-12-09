//! `Exchange` enum.
//!
//! This module defines the `Exchange` enum, which represents various exchange options available for trading.
//! It includes implementations for converting from and to different types, as well as helper methods
//! for determining exchange properties such as divisors and tradability.
//!
//! The `Exchange` enum supports serialization and deserialization using Serde, making it suitable
//! for use in JSON or other data formats.
//!
use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// Represents the exchanges venues supported by Kite Connect API.
///
/// This enum represents various exchange options available for trading.
/// Each variant corresponds to a specific exchange or market segment.
///
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub enum Exchange {
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

impl Exchange {
    /// Returns the divisor for the exchange.
    ///
    /// The divisor is used to normalize values based on the exchange.
    /// For `CDS` and `BCD`, specific divisors are returned, while a default divisor
    /// is used for other exchanges.
    ///
    /// # Returns
    ///
    /// A `f64` value representing the divisor.
    ///
    pub(crate) fn divisor(&self) -> f64 {
        match self {
            Self::CDS => 100_000_0.0,
            Self::BCD => 100_0.0,
            _ => 100.0,
        }
    }

    /// Determines if the exchange is tradable.
    ///
    /// The `INDICES` exchange is not tradable, while all other exchanges are.
    ///
    /// # Returns
    ///
    /// A `bool` indicating if the exchange is tradable.
    ///
    pub(crate) fn is_tradable(&self) -> bool {
        match self {
            Self::NONE => false,
            Self::INDICES => false,
            _ => true,
        }
    }
}

impl From<usize> for Exchange {
    /// Creates an `Exchange` from a `usize`.
    ///
    /// Maps integer values to specific exchanges. Values outside the predefined range
    /// default to `NSE`.
    ///
    /// # Arguments
    ///
    /// * `value` - A `usize` representing the exchange.
    ///
    /// # Returns
    ///
    /// An `Exchange` variant corresponding to the input value.
    ///
    fn from(value: usize) -> Self {
        match value {
            9 => Self::INDICES,
            8 => Self::MCXSX,
            7 => Self::MCX,
            6 => Self::BCD,
            5 => Self::BFO,
            4 => Self::BSE,
            3 => Self::CDS,
            2 => Self::NFO,
            1 => Self::NSE,
            _ => Self::NONE,
        }
    }
}

impl From<&str> for Exchange {
    /// Creates an `Exchange` from a `&str`.
    ///
    /// Maps string representations of exchange names to specific exchanges. Unrecognized
    /// strings default to `NSE`.
    ///
    /// # Arguments
    ///
    /// * `value` - An `&str` representing the exchange.
    ///
    /// # Returns
    ///
    /// An `Exchange` variant corresponding to the input string.
    ///
    fn from(value: &str) -> Self {
        match value {
            "NSE" => Self::NSE,
            "NFO" => Self::NFO,
            "CDS" => Self::CDS,
            "BSE" => Self::BSE,
            "BFO" => Self::BFO,
            "BCD" => Self::BCD,
            "MCX" => Self::MCX,
            "MCXSX" => Self::MCXSX,
            "INDICES" => Self::INDICES,
            "" => Self::NONE,
            _ => Self::NONE,
        }
    }
}

impl From<Exchange> for &str {
    /// Converts an `Exchange` to a `&str`.
    ///
    /// Maps each `Exchange` variant to its string representation.
    ///
    /// # Arguments
    ///
    /// * `value` - An `Exchange` variant.
    ///
    /// # Returns
    ///
    /// An `&str` corresponding to the exchange.
    ///
    fn from(value: Exchange) -> Self {
        match value {
            Exchange::NSE => "NSE",
            Exchange::NFO => "NFO",
            Exchange::CDS => "CDS",
            Exchange::BSE => "BSE",
            Exchange::BFO => "BFO",
            Exchange::BCD => "BCD",
            Exchange::MCX => "MCX",
            Exchange::MCXSX => "MCXSX",
            Exchange::INDICES => "INDICES",
            Exchange::NONE => "",
        }
    }
}

impl fmt::Display for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let display_str = match self {
            Exchange::NSE => "NSE",
            Exchange::NFO => "NFO",
            Exchange::CDS => "CDS",
            Exchange::BSE => "BSE",
            Exchange::BFO => "BFO",
            Exchange::BCD => "BCD",
            Exchange::MCX => "MCX",
            Exchange::MCXSX => "MCXSX",
            Exchange::INDICES => "INDICES",
            Exchange::NONE => "",
        };
        write!(f, "{}", display_str)
    }
}

impl<'de> Deserialize<'de> for Exchange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ExchangeVisitor;

        impl<'de> Visitor<'de> for ExchangeVisitor {
            type Value = Exchange;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a valid exchange string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Exchange, E>
            where
                E: de::Error,
            {
                Ok(match value {
                    "" => Exchange::NONE,
                    "NSE" => Exchange::NSE,
                    "NFO" => Exchange::NFO,
                    "CDS" => Exchange::CDS,
                    "BSE" => Exchange::BSE,
                    "BFO" => Exchange::BFO,
                    "BCD" => Exchange::BCD,
                    "MCX" => Exchange::MCX,
                    "MCXSX" => Exchange::MCXSX,
                    "INDICES" => Exchange::INDICES,
                    _ => Exchange::NONE,
                })
            }
        }

        deserializer.deserialize_str(ExchangeVisitor)
    }
}
