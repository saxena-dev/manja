//! Various quote and instrument related types.
//!
//! This module defines various types and structures related to trading
//! instruments and market data.
//!
//! It includes the definitions for different instrument types, trading
//! instruments, OHLC (Open, High, Low, Close) data, market depth levels, and
//! different modes of market quotes. These types are used for managing and
//! processing trading instruments and their market data within the application.
//!
use crate::kite::connect::models::exchange::Exchange;

use chrono::NaiveDate;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Represents the type of the instrument, such as `equity`, `futures` or `option`.
///
/// This enum contains several constant values used for specifying the type of instrument.
///
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum InstrumentType {
    /// Equity.
    #[serde(rename = "EQ")]
    Equity,

    /// Futures.
    #[serde(rename = "FUT")]
    Futures,

    /// Call Option.
    #[serde(rename = "CE")]
    CallOption,

    /// Put Option.
    #[serde(rename = "PE")]
    PutOption,
}

/// Represents a trading instrument.
///
/// Between multiple exchanges and segments, there are tens of thousands of
/// different kinds of instruments that trade. Any application that facilitates
/// trading needs to have a master list of these instruments. The instruments
/// API provides a consolidated, import-ready CSV list of instruments available
/// for trading.
///
/// # CSV response columns
///
/// - `instrument_token`: Numerical identifier used for subscribing to live market
///     quotes with the WebSocket API.
/// - `exchange_token`: The numerical identifier issued by the exchange representing
///     the instrument.
/// - `tradingsymbol`: Exchange tradingsymbol of the instrument.
/// - `name`: Name of the company (for equity instruments). This can be `None` for
///     non-equity instruments.
/// - `last_price`: Last traded market price.
/// - `expiry`: Expiry date (for derivatives). Optional because it may not be present
///     for some instruments.
/// - `strike`: Strike price (for options). Optional because it may not be present
///     for some instruments.
/// - `tick_size`: Value of a single price tick.
/// - `lot_size`: Quantity of a single lot.
/// - `instrument_type`: Type of the instrument (e.g., EQ, FUT, CE, PE).
/// - `segment`: Segment the instrument belongs to.
/// - `exchange`: Exchange where the instrument is traded.
///
#[derive(Debug, Deserialize, Clone)]
pub struct Instrument {
    /// Numerical identifier used for subscribing to live market quotes with the
    /// WebSocket API.
    pub instrument_token: i32,

    /// The numerical identifier issued by the exchange representing the instrument.
    pub exchange_token: String,

    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,

    /// Name of the company (for equity instruments). This can be `None` for
    /// non-equity instruments.
    pub name: Option<String>,

    /// Last traded market price.
    pub last_price: f64,

    /// Expiry date (for derivatives). Optional because it may not be present for
    /// some instruments.
    pub expiry: Option<NaiveDate>,

    /// Strike price (for options). Optional because it may not be present for
    /// some instruments.
    pub strike: Option<f64>,

    /// Value of a single price tick.
    pub tick_size: f64,

    /// Quantity of a single lot.
    pub lot_size: i64,

    /// Type of the instrument (e.g., EQ, FUT, CE, PE).
    pub instrument_type: InstrumentType,

    /// Segment the instrument belongs to.
    pub segment: String,

    /// Exchange where the instrument is traded.
    pub exchange: Exchange,

    // Cache quote format
    #[serde(skip)]
    query: Option<String>,
}

impl Instrument {
    /// Converts the instrument to a query string format used for market data requests.
    ///
    /// This method constructs a query string representation of the instrument,
    /// which can be used to request market data.
    ///
    /// # Returns
    ///
    /// A tuple containing the query key and the query string.
    ///
    pub fn to_query(&mut self) -> (&str, &str) {
        if let Some(ref query) = self.query {
            ("i", query.as_ref())
        } else {
            let symbol = self.tradingsymbol.replace(" ", "%20");
            self.query.replace(format!("{}:{}", self.exchange, symbol));
            self.to_query()
        }
    }
}

/// Represents the OHLC (Open, High, Low, Close) data of a market instrument.
///
#[derive(Debug, Serialize, Deserialize)]
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
#[derive(Debug, Serialize, Deserialize)]
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
#[derive(Debug, Serialize, Deserialize)]
pub struct Depth {
    /// The bid levels.
    pub buy: Vec<DepthLevel>,

    /// The ask levels.
    pub sell: Vec<DepthLevel>,
}

/// Represents the different modes of market quotes.
///
pub enum QuoteMode {
    Full,
    OHLC,
    LTP,
}

/// Trait for types that can be used as kite market quotes.
///
#[allow(unused)]
pub(crate) trait KiteQuote: DeserializeOwned {
    fn mode() -> QuoteMode;
}

/// Represents a market quote for an instrument, including OHLC, volume, and
/// market depth.
///
#[derive(Debug, Serialize, Deserialize)]
pub struct FullQuote {
    /// The numerical identifier issued by the exchange representing the instrument.
    pub instrument_token: u32,

    /// The exchange timestamp of the quote packet.
    pub timestamp: String,

    /// Last trade timestamp.
    pub last_trade_time: Option<String>,

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
#[derive(Debug, Serialize, Deserialize)]
pub struct OHLCQuote {
    /// The numerical identifier issued by the exchange representing the instrument.
    pub instrument_token: u32,

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
#[derive(Debug, Serialize, Deserialize)]
pub struct LTPQuote {
    /// The numerical identifier issued by the exchange representing the instrument.
    pub instrument_token: u32,

    /// Last traded market price.
    pub last_price: f64,
}

impl KiteQuote for LTPQuote {
    fn mode() -> QuoteMode {
        QuoteMode::LTP
    }
}
