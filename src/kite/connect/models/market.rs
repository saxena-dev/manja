//! Market quote and instrument types
//! (`kite-api-docs/docs/connect/v3/market-quotes.md`).
//!
//! Quotes are snapshots gathered at the time of the request. Instruments
//! come from the daily instrument dump: its `last_price` is not a live quote
//! (`market-quotes.md:17`), and an instrument token may be reused for a
//! different derivative after expiry, so `(exchange, tradingsymbol)` is the
//! documented storage key (`market-quotes.md:54-56`). This crate owns no
//! instrument registry.
//!
use std::collections::HashMap;

use crate::kite::connect::models::exchange::Exchange;
use crate::kite::protocol::datetime::serde_opt_datetime;
use crate::kite::protocol::enums::wire_enum;
use crate::kite::protocol::{Inbound, InstrumentToken};

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The type of an instrument (`market-quotes.md:46`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InstrumentType {
    /// Equity.
    Equity,
    /// Futures.
    Futures,
    /// Call option.
    CallOption,
    /// Put option.
    PutOption,
}

wire_enum!(InstrumentType {
    Equity => "EQ",
    Futures => "FUT",
    CallOption => "CE",
    PutOption => "PE",
});

/// One row of the instrument dump (`market-quotes.md:33-48`).
///
/// `last_price` is from the daily dump and is not a live quote. An
/// `instrument_token` identifies the instrument only while it trades;
/// exchanges may reuse tokens after expiry.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Instrument {
    /// Token for WebSocket subscriptions.
    pub instrument_token: InstrumentToken,
    /// The exchange's own identifier of the instrument.
    pub exchange_token: String,
    /// Exchange tradingsymbol.
    pub tradingsymbol: String,
    /// Company name, for equities; `None` when empty.
    pub name: Option<String>,
    /// Last traded price at the time of the daily dump; not a live quote.
    pub last_price: f64,
    /// Expiry date, for derivatives; `None` when empty.
    pub expiry: Option<NaiveDate>,
    /// Strike price, for options.
    pub strike: Option<f64>,
    /// Value of a single price tick.
    pub tick_size: f64,
    /// Quantity of a single lot.
    pub lot_size: i64,
    /// EQ, FUT, CE, PE, or an unknown type preserved as received.
    pub instrument_type: Inbound<InstrumentType>,
    /// Segment, such as `NSE` or `NFO-OPT`.
    pub segment: String,
    /// Exchange.
    pub exchange: Inbound<Exchange>,
}

impl Instrument {
    /// The quote key of the instrument, `EXCHANGE:TRADINGSYMBOL`
    /// (`market-quotes.md:66`).
    pub fn quote_key(&self) -> String {
        format!("{}:{}", self.exchange.as_wire(), self.tradingsymbol)
    }
}

/// Quotes for a set of requested instruments.
///
/// "If there is no data available for a given key, the key will be absent
/// from the response" (`market-quotes.md:66`): such instruments are listed in
/// [`Self::missing`], never filled with a zero quote. One request is one
/// snapshot; quotes of different requests are not an atomic set.
#[derive(Clone, Debug, PartialEq)]
pub struct Quotes<Q> {
    /// Instrument keys requested, in request order.
    pub requested: Vec<String>,
    /// Quotes received, by instrument key.
    pub received: HashMap<String, Q>,
    /// Requested keys absent from the response, in request order.
    pub missing: Vec<String>,
    /// Keys in the response that were not requested, if any.
    pub unexpected: Vec<String>,
}

impl<Q> Quotes<Q> {
    /// The quote of `key`, or `None` if it was missing.
    pub fn get(&self, key: &str) -> Option<&Q> {
        self.received.get(key)
    }

    /// Whether every requested key received a quote.
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Represents the OHLC (Open, High, Low, Close) data of a market instrument.
///
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)] // Public name kept for compatibility.
pub struct OHLC {
    /// Price at market opening.
    pub open: f64,

    /// Highest price today.
    pub high: f64,

    /// Lowest price today.
    pub low: f64,

    /// Closing price of the instrument from the last trading day.
    pub close: f64,
}

/// Represents a depth level in the order book for an instrument.
///
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DepthLevel {
    /// Price at which the depth stands.
    pub price: f64,

    /// Number of open orders at the price.
    pub orders: i64,

    /// Net quantity from the pending orders.
    pub quantity: i64,
}

/// Represents the market depth for an instrument, including bid and ask levels.
///
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Depth {
    /// The bid levels.
    pub buy: Vec<DepthLevel>,

    /// The ask levels.
    pub sell: Vec<DepthLevel>,
}

/// The quote endpoints and their documented request limits
/// (`market-quotes.md:272-278`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(clippy::upper_case_acronyms)] // Public names kept for compatibility.
pub enum QuoteMode {
    /// `/quote`: full quotes, at most 500 instruments.
    Full,
    /// `/quote/ohlc`: OHLC and LTP, at most 1000 instruments.
    OHLC,
    /// `/quote/ltp`: LTP, at most 1000 instruments.
    LTP,
}

impl QuoteMode {
    /// Endpoint path.
    pub fn path(self) -> &'static str {
        match self {
            Self::Full => "/quote",
            Self::OHLC => "/quote/ohlc",
            Self::LTP => "/quote/ltp",
        }
    }

    /// Maximum instruments per request.
    pub fn max_instruments(self) -> usize {
        match self {
            Self::Full => 500,
            Self::OHLC | Self::LTP => 1000,
        }
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::FullQuote {}
    impl Sealed for super::OHLCQuote {}
    impl Sealed for super::LTPQuote {}
}

/// A quote type: [`FullQuote`], [`OHLCQuote`] or [`LTPQuote`].
///
/// Sealed: each implementation is tied to one documented endpoint, so no
/// other type can implement it.
pub trait KiteQuote: DeserializeOwned + sealed::Sealed {
    /// The endpoint serving this quote type.
    fn mode() -> QuoteMode;
}

/// Represents a market quote for an instrument, including OHLC, volume, and
/// market depth.
///
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FullQuote {
    /// Instrument token.
    pub instrument_token: InstrumentToken,

    /// The exchange timestamp of the quote (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub timestamp: Option<DateTime<FixedOffset>>,

    /// Last trade timestamp (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub last_trade_time: Option<DateTime<FixedOffset>>,

    /// Last traded market price.
    pub last_price: f64,

    /// Volume traded today.
    pub volume: Option<i64>,

    /// The volume weighted average price of a stock at a given time during the day.
    pub average_price: Option<f64>,

    /// Total quantity of buy orders pending at the exchange.
    pub buy_quantity: Option<i64>,

    /// Total quantity of sell orders pending at the exchange.
    pub sell_quantity: Option<i64>,

    /// Total number of outstanding contracts held by market participants
    /// exchange-wide (only F&O).
    pub open_interest: Option<f64>,

    /// Last traded quantity.
    pub last_quantity: Option<i64>,

    /// OHLC data.
    pub ohlc: OHLC,

    /// The absolute change from yesterday's close to last traded price.
    pub net_change: f64,

    /// The current lower circuit limit.
    pub lower_circuit_limit: Option<f64>,

    /// The current upper circuit limit.
    pub upper_circuit_limit: Option<f64>,

    /// The Open Interest for a futures or options contract.
    pub oi: Option<f64>,

    /// The highest Open Interest recorded during the day.
    pub oi_day_high: Option<f64>,

    /// The lowest Open Interest recorded during the day.
    pub oi_day_low: Option<f64>,

    /// Market depth data.
    pub depth: Option<Depth>,
}

impl KiteQuote for FullQuote {
    fn mode() -> QuoteMode {
        QuoteMode::Full
    }
}

/// Represents an OHLC + LTP quote for an instrument, including OHLC, volume,
/// and market depth.
///
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OHLCQuote {
    /// Instrument token.
    pub instrument_token: InstrumentToken,

    /// Last traded market price.
    pub last_price: f64,

    /// OHLC data.
    pub ohlc: OHLC,
}

impl KiteQuote for OHLCQuote {
    fn mode() -> QuoteMode {
        QuoteMode::OHLC
    }
}

/// Represents an LTP quote for an instrument, including OHLC, volume, and
/// market depth.
///
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LTPQuote {
    /// Instrument token.
    pub instrument_token: InstrumentToken,

    /// Last traded market price.
    pub last_price: f64,
}

impl KiteQuote for LTPQuote {
    fn mode() -> QuoteMode {
        QuoteMode::LTP
    }
}
