//! Historical OHLCV(+OI) candle models.
//!
//! This module defines typed representations for historical candles returned
//! by the Kite Connect API, along with the supported interval enum. The HTTP
//! APIs expose these types via the `manja-http` transport crate.

use std::fmt;

use chrono::{DateTime, FixedOffset};
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// Supported historical candle intervals.
///
/// These values map directly to the `:interval` URI parameter used by the
/// `/instruments/historical/:instrument_token/:interval` endpoint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HistoricalInterval {
    /// 1-minute candles (`minute`).
    #[serde(rename = "minute")]
    Minute,
    /// 3-minute candles (`3minute`).
    #[serde(rename = "3minute")]
    ThreeMinute,
    /// 5-minute candles (`5minute`).
    #[serde(rename = "5minute")]
    FiveMinute,
    /// 10-minute candles (`10minute`).
    #[serde(rename = "10minute")]
    TenMinute,
    /// 15-minute candles (`15minute`).
    #[serde(rename = "15minute")]
    FifteenMinute,
    /// 30-minute candles (`30minute`).
    #[serde(rename = "30minute")]
    ThirtyMinute,
    /// 60-minute candles (`60minute`).
    #[serde(rename = "60minute")]
    SixtyMinute,
    /// Daily candles (`day`).
    #[serde(rename = "day")]
    Day,
}

impl HistoricalInterval {
    /// Returns the Kite Connect string representation for this interval.
    pub fn as_str(&self) -> &'static str {
        match self {
            HistoricalInterval::Minute => "minute",
            HistoricalInterval::ThreeMinute => "3minute",
            HistoricalInterval::FiveMinute => "5minute",
            HistoricalInterval::TenMinute => "10minute",
            HistoricalInterval::FifteenMinute => "15minute",
            HistoricalInterval::ThirtyMinute => "30minute",
            HistoricalInterval::SixtyMinute => "60minute",
            HistoricalInterval::Day => "day",
        }
    }
}

impl fmt::Display for HistoricalInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single historical OHLCV(+OI) candle.
///
/// The Kite Connect Historical API returns an array-of-arrays representation:
///
/// ```text
/// [
///   "2017-12-15T09:15:00+0530",
///   1704.5,
///   1705.0,
///   1699.25,
///   1702.8,
///   2499,
///   13667775 // optional OI
/// ]
/// ```
///
/// This type provides a typed view over that representation.
#[derive(Debug, Clone, Serialize)]
pub struct HistoricalCandle {
    /// Candle timestamp with timezone offset.
    pub timestamp: DateTime<FixedOffset>,
    /// Open price.
    pub open: f64,
    /// High price.
    pub high: f64,
    /// Low price.
    pub low: f64,
    /// Close price.
    pub close: f64,
    /// Traded volume during the interval.
    pub volume: i64,
    /// Open interest during the interval, when requested with `oi=1`.
    pub oi: Option<i64>,
}

impl<'de> Deserialize<'de> for HistoricalCandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CandleVisitor;

        impl<'de> Visitor<'de> for CandleVisitor {
            type Value = HistoricalCandle;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a historical candle array")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let ts: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let open: f64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let high: f64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let low: f64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                let close: f64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(4, &self))?;
                let volume: i64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(5, &self))?;
                // OI is only present when `oi=1` is requested.
                let oi: Option<i64> = seq.next_element()?;

                let timestamp = DateTime::parse_from_str(&ts, "%Y-%m-%dT%H:%M:%S%z")
                    .map_err(|err| {
                        de::Error::custom(format!("invalid candle timestamp `{}`: {}", ts, err))
                    })?;

                Ok(HistoricalCandle {
                    timestamp,
                    open,
                    high,
                    low,
                    close,
                    volume,
                    oi,
                })
            }
        }

        deserializer.deserialize_seq(CandleVisitor)
    }
}

/// Container for historical candles as returned by Kite Connect.
///
/// The HTTP response is wrapped in `KiteApiResponse<HistoricalData>`.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoricalData {
    /// The sequence of OHLCV(+OI) candles.
    pub candles: Vec<HistoricalCandle>,
}

