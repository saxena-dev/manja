//! Broker datetime parsing.
//!
//! Kite represents timestamps as offset-free `yyyy-mm-dd hh:mm:ss` strings
//! in Indian Standard Time, UTC+05:30, and dates as `yyyy-mm-dd`
//! (`kite:response-structure.md:33-35`).
//! [`parse_broker_datetime`] is the crate's single parser for that form: it
//! parses the text as a naive local time and then attaches the explicit
//! +05:30 offset. It never guesses: an offset-bearing, `T`-separated, empty or
//! malformed string is an error, and `null` stays `None` in the optional
//! serde helpers.
//!
//! Some responses carry a bare `hh:mm:ss` time of day in a field documented
//! as a timestamp (the official `trades.json` and `order_trades.json`
//! fixtures' `order_timestamp`). [`BrokerTimestamp`] represents that shape
//! explicitly instead of inventing a date.
//!
use std::fmt;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The IST offset, in seconds east of UTC.
pub const IST_OFFSET_SECONDS: i32 = 5 * 3600 + 30 * 60;

const DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
const DATE_FORMAT: &str = "%Y-%m-%d";
const TIME_FORMAT: &str = "%H:%M:%S";

fn ist() -> FixedOffset {
    FixedOffset::east_opt(IST_OFFSET_SECONDS).expect("+05:30 is a valid offset")
}

/// A broker datetime or date string did not have the documented form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateTimeError {
    input_len: usize,
    expected: &'static str,
}

impl fmt::Display for DateTimeError {
    // The rejected text is not echoed; only its length and the expected form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "broker timestamp of {} bytes is not in the form {}",
            self.input_len, self.expected
        )
    }
}

impl std::error::Error for DateTimeError {}

/// Parse `yyyy-mm-dd hh:mm:ss` as naive IST, then attach UTC+05:30.
///
/// ```
/// use manja::kite::protocol::parse_broker_datetime;
///
/// let t = parse_broker_datetime("2022-03-03 09:24:25").unwrap();
/// assert_eq!(t.to_rfc3339(), "2022-03-03T09:24:25+05:30");
/// assert!(parse_broker_datetime("2022-03-03T09:24:25+05:30").is_err());
/// ```
pub fn parse_broker_datetime(s: &str) -> Result<DateTime<FixedOffset>, DateTimeError> {
    let naive = NaiveDateTime::parse_from_str(s, DATETIME_FORMAT).map_err(|_| DateTimeError {
        input_len: s.len(),
        expected: "yyyy-mm-dd hh:mm:ss",
    })?;
    // A fixed offset has exactly one mapping for every local time.
    Ok(ist()
        .from_local_datetime(&naive)
        .single()
        .expect("fixed offsets are unambiguous"))
}

/// Parse a broker date, `yyyy-mm-dd`.
pub fn parse_broker_date(s: &str) -> Result<NaiveDate, DateTimeError> {
    NaiveDate::parse_from_str(s, DATE_FORMAT).map_err(|_| DateTimeError {
        input_len: s.len(),
        expected: "yyyy-mm-dd",
    })
}

fn format_broker_datetime(t: &DateTime<FixedOffset>) -> String {
    t.with_timezone(&ist()).format(DATETIME_FORMAT).to_string()
}

/// A timestamp field that is either a full broker datetime or a bare time of
/// day, as both occur in documented responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrokerTimestamp {
    /// A full `yyyy-mm-dd hh:mm:ss` datetime in IST.
    DateTime(DateTime<FixedOffset>),
    /// A bare `hh:mm:ss` time of day in IST; the date is not supplied.
    TimeOfDay(NaiveTime),
}

impl BrokerTimestamp {
    /// Parse either documented shape.
    pub fn parse(s: &str) -> Result<Self, DateTimeError> {
        if let Ok(t) = parse_broker_datetime(s) {
            return Ok(Self::DateTime(t));
        }
        NaiveTime::parse_from_str(s, TIME_FORMAT)
            .map(Self::TimeOfDay)
            .map_err(|_| DateTimeError {
                input_len: s.len(),
                expected: "yyyy-mm-dd hh:mm:ss or hh:mm:ss",
            })
    }

    /// The full datetime, when one was supplied.
    pub fn datetime(&self) -> Option<DateTime<FixedOffset>> {
        match self {
            Self::DateTime(t) => Some(*t),
            Self::TimeOfDay(_) => None,
        }
    }
}

impl Serialize for BrokerTimestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::DateTime(t) => s.serialize_str(&format_broker_datetime(t)),
            Self::TimeOfDay(t) => s.serialize_str(&t.format(TIME_FORMAT).to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for BrokerTimestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Serde helpers for a required `DateTime<FixedOffset>` broker field.
pub mod serde_datetime {
    use super::*;

    /// Serialize in the broker form.
    pub fn serialize<S: Serializer>(t: &DateTime<FixedOffset>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format_broker_datetime(t))
    }

    /// Deserialize from the broker form; anything else is an error.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<FixedOffset>, D::Error> {
        let s = String::deserialize(d)?;
        parse_broker_datetime(&s).map_err(serde::de::Error::custom)
    }
}

/// Serde helpers for an optional broker datetime field: `null` (or an absent
/// field, with `#[serde(default)]`) is `None`; any string must parse.
pub mod serde_opt_datetime {
    use super::*;

    /// Serialize `None` as `null`, otherwise in the broker form.
    pub fn serialize<S: Serializer>(
        t: &Option<DateTime<FixedOffset>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => s.serialize_str(&format_broker_datetime(t)),
            None => s.serialize_none(),
        }
    }

    /// Deserialize `null` as `None`; a string must be in the broker form.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<DateTime<FixedOffset>>, D::Error> {
        match Option::<String>::deserialize(d)? {
            Some(s) => parse_broker_datetime(&s)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

/// Serde helpers for an optional broker date field, `yyyy-mm-dd`: `null`,
/// an absent field (with `#[serde(default)]`) or an empty string is `None`;
/// any other string must parse. The official mutual fund holdings send an
/// empty `last_price_date`.
pub mod serde_opt_date {
    use super::*;

    /// Serialize `None` as `null`, otherwise as `yyyy-mm-dd`.
    pub fn serialize<S: Serializer>(d: &Option<NaiveDate>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_str(&d.format(DATE_FORMAT).to_string()),
            None => s.serialize_none(),
        }
    }

    /// Deserialize `null` or `""` as `None`; any other string must be a date.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<NaiveDate>, D::Error> {
        match Option::<String>::deserialize(d)? {
            Some(s) if !s.is_empty() => parse_broker_date(&s)
                .map(Some)
                .map_err(serde::de::Error::custom),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_dates_accept_null_empty_and_the_documented_form() {
        #[derive(Debug, Deserialize, Serialize)]
        struct D {
            #[serde(with = "serde_opt_date", default)]
            on: Option<NaiveDate>,
        }
        let d: D = serde_json::from_str(r#"{"on": "2021-06-29"}"#).unwrap();
        assert_eq!(d.on, NaiveDate::from_ymd_opt(2021, 6, 29));
        assert_eq!(serde_json::to_string(&d).unwrap(), r#"{"on":"2021-06-29"}"#);
        for none in [r#"{"on": null}"#, r#"{"on": ""}"#, "{}"] {
            assert_eq!(serde_json::from_str::<D>(none).unwrap().on, None, "{none}");
        }
        assert!(serde_json::from_str::<D>(r#"{"on": "29-06-2021"}"#).is_err());
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct Row {
        #[serde(with = "serde_opt_datetime", default)]
        at: Option<DateTime<FixedOffset>>,
    }

    #[test]
    fn documented_form_is_naive_ist() {
        let t = parse_broker_datetime("2021-05-31 09:18:57").unwrap();
        assert_eq!(t.offset().local_minus_utc(), IST_OFFSET_SECONDS);
        assert_eq!(t.naive_local().to_string(), "2021-05-31 09:18:57");
        assert_eq!(t.to_utc().to_rfc3339(), "2021-05-31T03:48:57+00:00");
    }

    #[test]
    fn null_stays_null_and_bad_strings_fail_visibly() {
        let row: Row = serde_json::from_str(r#"{"at": null}"#).unwrap();
        assert!(row.at.is_none());
        let row: Row = serde_json::from_str("{}").unwrap();
        assert!(row.at.is_none());
        for bad in [
            r#"{"at": ""}"#,
            r#"{"at": "2021-05-31T09:18:57+05:30"}"#,
            r#"{"at": "2021-05-31 09:18:57+05:30"}"#,
            r#"{"at": "2021-13-01 09:18:57"}"#,
            r#"{"at": "31-05-2021 09:18:57"}"#,
            r#"{"at": 1622434137}"#,
        ] {
            let err = serde_json::from_str::<Row>(bad).unwrap_err();
            assert!(!err.to_string().is_empty(), "{bad}");
        }
    }

    #[test]
    fn serialization_round_trips_the_broker_form() {
        let row: Row = serde_json::from_str(r#"{"at": "2022-03-03 09:24:25"}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&row).unwrap(),
            r#"{"at":"2022-03-03 09:24:25"}"#
        );
    }

    #[test]
    fn time_of_day_is_an_explicit_variant() {
        let t = BrokerTimestamp::parse("09:16:39").unwrap();
        assert!(matches!(t, BrokerTimestamp::TimeOfDay(_)));
        assert!(t.datetime().is_none());
        let t = BrokerTimestamp::parse("2021-05-31 09:16:39").unwrap();
        assert!(t.datetime().is_some());
        assert!(BrokerTimestamp::parse("9:16").is_err());
    }

    #[test]
    fn dates_parse_separately() {
        assert_eq!(
            parse_broker_date("2015-12-31").unwrap().to_string(),
            "2015-12-31"
        );
        assert!(parse_broker_date("2015-12-31 00:00:00").is_err());
    }
}
