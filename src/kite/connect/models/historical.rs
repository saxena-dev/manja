//! Historical candle types (`kite:historical.md`).
//!
//! A candle is one interval's open, high, low, close and volume, and, when
//! asked for, open interest. The broker sends each candle as a JSON array,
//! `[timestamp, open, high, low, close, volume]` with an optional seventh
//! value for open interest (`kite:historical.md:25-27,114-185`), so [`Candle`]
//! decodes that array by position. A candle with fewer than six values, more
//! than seven, or a value of the wrong type fails the response.
//!
//! Candle timestamps carry their own offset (`2017-12-15T09:15:00+0530`),
//! which is kept as sent.
//!
use std::fmt;

use chrono::{DateTime, FixedOffset};
use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::kite::connect::models::order::{RequestError, invalid};
use crate::kite::protocol::InstrumentToken;
use crate::kite::protocol::enums::wire_enum;

/// The interval of a candle (`kite:historical.md:14`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CandleInterval {
    /// One minute.
    Minute,
    /// Three minutes.
    ThreeMinute,
    /// Five minutes.
    FiveMinute,
    /// Ten minutes.
    TenMinute,
    /// Fifteen minutes.
    FifteenMinute,
    /// Thirty minutes.
    ThirtyMinute,
    /// Sixty minutes.
    SixtyMinute,
    /// One trading day.
    Day,
}

wire_enum!(CandleInterval {
    Minute => "minute",
    ThreeMinute => "3minute",
    FiveMinute => "5minute",
    TenMinute => "10minute",
    FifteenMinute => "15minute",
    ThirtyMinute => "30minute",
    SixtyMinute => "60minute",
    Day => "day",
});

/// The candles of a historical data response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoricalData {
    /// Candles in the order the broker sent them.
    pub candles: Vec<Candle>,
}

/// One candle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candle {
    /// Start of the interval, with the offset the broker sent.
    pub timestamp: DateTime<FixedOffset>,
    /// Opening price.
    pub open: f64,
    /// Highest price.
    pub high: f64,
    /// Lowest price.
    pub low: f64,
    /// Closing price.
    pub close: f64,
    /// Traded volume.
    pub volume: u64,
    /// Open interest, present when the request asked for it.
    pub oi: Option<u64>,
}

/// The timestamp form of candles.
const CANDLE_TIMESTAMP: &str = "%Y-%m-%dT%H:%M:%S%z";

impl<'de> Deserialize<'de> for Candle {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct CandleVisitor;

        impl<'de> Visitor<'de> for CandleVisitor {
            type Value = Candle;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "[timestamp, open, high, low, close, volume] and optional open interest",
                )
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Candle, A::Error> {
                fn need<'de, A: SeqAccess<'de>, T: Deserialize<'de>>(
                    seq: &mut A,
                    index: usize,
                ) -> Result<T, A::Error> {
                    seq.next_element()?
                        .ok_or_else(|| de::Error::invalid_length(index, &"at least six values"))
                }
                let raw: String = need(&mut seq, 0)?;
                let timestamp = DateTime::parse_from_str(&raw, CANDLE_TIMESTAMP).map_err(|_| {
                    de::Error::invalid_value(de::Unexpected::Str(&raw), &"a timestamp with offset")
                })?;
                let candle = Candle {
                    timestamp,
                    open: need(&mut seq, 1)?,
                    high: need(&mut seq, 2)?,
                    low: need(&mut seq, 3)?,
                    close: need(&mut seq, 4)?,
                    volume: need(&mut seq, 5)?,
                    // A seventh value of `null` is no open interest.
                    oi: seq.next_element::<Option<u64>>()?.flatten(),
                };
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::invalid_length(8, &"at most seven values"));
                }
                Ok(candle)
            }
        }

        d.deserialize_seq(CandleVisitor)
    }
}

impl Serialize for Candle {
    /// The broker's array form, so a serialized candle reads back.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(if self.oi.is_some() { 7 } else { 6 }))?;
        seq.serialize_element(&self.timestamp.format(CANDLE_TIMESTAMP).to_string())?;
        seq.serialize_element(&self.open)?;
        seq.serialize_element(&self.high)?;
        seq.serialize_element(&self.low)?;
        seq.serialize_element(&self.close)?;
        seq.serialize_element(&self.volume)?;
        if let Some(oi) = self.oi {
            seq.serialize_element(&oi)?;
        }
        seq.end()
    }
}

// --- [ Request DTOs ] ---

/// A historical data request:
/// `GET /instruments/historical/{instrument_token}/{interval}` with `from`,
/// `to`, `continuous` and `oi` query parameters
/// (`kite:historical.md:5-23`).
///
/// `from` and `to` are sent as IST wall time in the documented
/// `yyyy-mm-dd hh:mm:ss` form, whatever offset they carry here, so an instant
/// given in another zone is converted rather than reinterpreted.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoricalRequest {
    /// The instrument, from the instrument master.
    pub instrument_token: InstrumentToken,
    /// Candle interval.
    pub interval: CandleInterval,
    /// Start of the range.
    pub from: DateTime<FixedOffset>,
    /// End of the range; not before `from`.
    pub to: DateTime<FixedOffset>,
    /// Continuous data across expired futures contracts
    /// (`kite:historical.md:35-39`).
    pub continuous: bool,
    /// Include open interest in every candle.
    pub oi: bool,
}

impl HistoricalRequest {
    /// A request for `interval` candles of `instrument_token` between
    /// `from` and `to`, without continuous data or open interest.
    pub fn new(
        instrument_token: InstrumentToken,
        interval: CandleInterval,
        from: DateTime<FixedOffset>,
        to: DateTime<FixedOffset>,
    ) -> Self {
        Self {
            instrument_token,
            interval,
            from,
            to,
            continuous: false,
            oi: false,
        }
    }

    /// Check the range: `to` must not be before `from`. The documentation
    /// gives no maximum range per interval, so none is enforced here; a
    /// response larger than the JSON body bound (`B-HTTP-08`) is a `Decode`
    /// error.
    pub fn validate(&self) -> Result<(), RequestError> {
        if self.to < self.from {
            return invalid("to", "is before from");
        }
        Ok(())
    }

    /// The request path.
    #[cfg_attr(not(feature = "http"), allow(dead_code))]
    pub(crate) fn path(&self) -> String {
        format!(
            "/instruments/historical/{}/{}",
            self.instrument_token.get(),
            self.interval
        )
    }

    /// Query parameters in a fixed order. `continuous` and `oi` are sent
    /// only when set, as in the documented examples
    /// (`kite:historical.md:51,118`).
    #[cfg_attr(not(feature = "http"), allow(dead_code))]
    pub(crate) fn query(&self) -> Vec<(&'static str, String)> {
        let ist = FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("a valid offset");
        let form = |t: &DateTime<FixedOffset>| {
            t.with_timezone(&ist)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        };
        let mut q = vec![("from", form(&self.from)), ("to", form(&self.to))];
        if self.continuous {
            q.push(("continuous", "1".to_string()));
        }
        if self.oi {
            q.push(("oi", "1".to_string()));
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ist(h: u32, m: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(19800)
            .unwrap()
            .with_ymd_and_hms(2017, 12, 15, h, m, 0)
            .unwrap()
    }

    #[test]
    fn candles_decode_with_and_without_open_interest() {
        let c: Candle =
            serde_json::from_str(r#"["2017-12-15T09:15:00+0530",1704.5,1705,1699.25,1702.8,2499]"#)
                .unwrap();
        assert_eq!(c.timestamp, ist(9, 15));
        assert_eq!(
            (c.open, c.high, c.low, c.close),
            (1704.5, 1705.0, 1699.25, 1702.8)
        );
        assert_eq!((c.volume, c.oi), (2499, None));
        let c: Candle = serde_json::from_str(
            r#"["2019-12-04T09:15:00+0530",12009.9,12019.35,12001.25,12001.5,163275,13667775]"#,
        )
        .unwrap();
        assert_eq!((c.volume, c.oi), (163275, Some(13667775)));
        // Round trip in the broker's form.
        let back: Candle = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        let c: Candle =
            serde_json::from_str(r#"["2019-12-04T09:15:00+0530",1,2,0.5,1.5,10,null]"#).unwrap();
        assert_eq!(c.oi, None);
    }

    #[test]
    fn malformed_candles_are_errors() {
        for bad in [
            r#"["2017-12-15T09:15:00+0530",1,2,3,4]"#,
            r#"["2017-12-15T09:15:00+0530",1,2,3,4,5,6,7]"#,
            r#"["2017-12-15 09:15:00",1,2,3,4,5]"#,
            r#"["2017-12-15T09:15:00+0530","1",2,3,4,5]"#,
            r#"["2017-12-15T09:15:00+0530",1,2,3,4,-5]"#,
            r#"{"timestamp":"2017-12-15T09:15:00+0530"}"#,
        ] {
            assert!(serde_json::from_str::<Candle>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_query_is_ist_wall_time_in_the_documented_form() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let from = utc.with_ymd_and_hms(2017, 12, 15, 3, 45, 0).unwrap();
        let mut r = HistoricalRequest::new(
            InstrumentToken::new(5633),
            CandleInterval::Minute,
            from,
            ist(9, 20),
        );
        assert_eq!(r.path(), "/instruments/historical/5633/minute");
        assert_eq!(
            r.query(),
            vec![
                ("from", "2017-12-15 09:15:00".to_string()),
                ("to", "2017-12-15 09:20:00".to_string()),
            ]
        );
        r.continuous = true;
        r.oi = true;
        assert_eq!(
            r.query()[2..],
            [("continuous", "1".to_string()), ("oi", "1".to_string())]
        );
        assert_eq!(r.validate(), Ok(()));
        r.to = ist(9, 0);
        assert_eq!(r.validate().unwrap_err().field, "to");
    }
}
