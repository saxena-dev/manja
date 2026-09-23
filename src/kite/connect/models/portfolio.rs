//! Portfolio types: holdings, holdings auctions, positions and position
//! conversion (`kite-api-docs/docs/connect/v3/portfolio.md`).
//!
//! Holdings, auctions and positions are broker snapshots at the time of the
//! request, not reconciled account state. Exchange and product strings are
//! [`Inbound`] values: the official `positions.json` fixture contains the
//! product `CO`, which is preserved rather than rejected.
//!
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::kite::connect::models::{
    exchange::Exchange,
    order_enums::{ProductType, TransactionType},
};
use crate::kite::protocol::datetime::serde_opt_datetime;
use crate::kite::protocol::{Inbound, InstrumentToken};

/// Margin Trading Facility details of a holding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoldingMtf {
    /// MTF quantity.
    pub quantity: i64,
    /// MTF quantity used.
    pub used_quantity: i64,
    /// Average MTF price.
    pub average_price: f64,
    /// MTF value.
    pub value: f64,
    /// Initial margin.
    pub initial_margin: f64,
}

/// A long-term equity delivery holding in the user's demat account.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Holding {
    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,
    /// Exchange.
    pub exchange: Inbound<Exchange>,
    /// Instrument token.
    pub instrument_token: InstrumentToken,
    /// ISIN.
    pub isin: String,
    /// Margin product applied to the holding.
    pub product: Inbound<ProductType>,
    /// The price of the instrument.
    pub price: f64,
    /// Net quantity (T+1 + realised).
    pub quantity: i64,
    /// Quantity sold from the net holding quantity.
    pub used_quantity: i64,
    /// Quantity on T+1 day after order execution.
    pub t1_quantity: i64,
    /// Quantity delivered to demat.
    pub realised_quantity: i64,
    /// Quantity authorized at the depository for sale.
    pub authorised_quantity: i64,
    /// Date of the depository authorisation (IST), if any.
    #[serde(default, with = "serde_opt_datetime")]
    pub authorised_date: Option<DateTime<FixedOffset>>,
    /// Authorisation details, as returned.
    #[serde(default)]
    pub authorisation: Option<serde_json::Value>,
    /// Quantity carried forward overnight.
    pub opening_quantity: i64,
    /// Short quantity.
    #[serde(default)]
    pub short_quantity: Option<i64>,
    /// Quantity used as collateral.
    pub collateral_quantity: i64,
    /// Type of collateral; may be empty.
    #[serde(default)]
    pub collateral_type: Option<String>,
    /// Whether the holding has a price discrepancy.
    pub discrepancy: bool,
    /// Average price at which the net holding quantity was acquired.
    pub average_price: f64,
    /// Last traded market price of the instrument.
    pub last_price: f64,
    /// Closing price of the instrument from the last trading day.
    pub close_price: f64,
    /// Net returns on the stock.
    pub pnl: f64,
    /// Day's change in absolute value for the stock.
    pub day_change: f64,
    /// Day's change in percentage for the stock.
    pub day_change_percentage: f64,
    /// Margin Trading Facility details, if any.
    #[serde(default)]
    pub mtf: Option<HoldingMtf>,
}

/// A holding currently offered in an auction
/// (`kite-api-docs/docs/connect/v3/portfolio.md:123-217`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Auction {
    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,
    /// Exchange.
    pub exchange: Inbound<Exchange>,
    /// Instrument token.
    pub instrument_token: InstrumentToken,
    /// ISIN.
    pub isin: String,
    /// Margin product applied to the holding.
    pub product: Inbound<ProductType>,
    /// The price of the instrument.
    pub price: f64,
    /// Net quantity (T+1 + realised).
    pub quantity: i64,
    /// Quantity on T+1 day after order execution.
    pub t1_quantity: i64,
    /// Quantity delivered to demat.
    pub realised_quantity: i64,
    /// Quantity authorized at the depository for sale.
    pub authorised_quantity: i64,
    /// Date of the depository authorisation (IST), if any.
    #[serde(default, with = "serde_opt_datetime")]
    pub authorised_date: Option<DateTime<FixedOffset>>,
    /// Quantity carried forward overnight.
    pub opening_quantity: i64,
    /// Quantity used as collateral.
    pub collateral_quantity: i64,
    /// Type of collateral; may be empty.
    #[serde(default)]
    pub collateral_type: Option<String>,
    /// Whether the holding has a price discrepancy.
    pub discrepancy: bool,
    /// Average price at which the net holding quantity was acquired.
    pub average_price: f64,
    /// Last traded market price of the instrument.
    pub last_price: f64,
    /// Closing price of the instrument from the last trading day.
    pub close_price: f64,
    /// Net returns on the stock.
    pub pnl: f64,
    /// Day's change in absolute value for the stock.
    pub day_change: f64,
    /// Day's change in percentage for the stock.
    pub day_change_percentage: f64,
    /// Identifier of the auction.
    pub auction_number: String,
}

/// The positions response: two sets of positions
/// (`kite-api-docs/docs/connect/v3/portfolio.md:219-235`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Positions {
    /// The actual, current net position portfolio.
    pub net: Vec<Position>,
    /// A snapshot of the buying and selling activity of the day.
    pub day: Vec<Position>,
}

/// One position in the `net` or `day` set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,
    /// Exchange.
    pub exchange: Inbound<Exchange>,
    /// Instrument token.
    pub instrument_token: InstrumentToken,
    /// Margin product applied to the position.
    pub product: Inbound<ProductType>,
    /// Quantity held.
    pub quantity: i64,
    /// Quantity held previously and carried forward overnight.
    pub overnight_quantity: i64,
    /// The quantity/lot size multiplier used for calculating P&Ls.
    pub multiplier: i64,
    /// Average price at which the net position quantity was acquired.
    pub average_price: f64,
    /// Closing price of the instrument from the last trading day.
    pub close_price: f64,
    /// Last traded market price of the instrument.
    pub last_price: f64,
    /// Net value of the position.
    pub value: f64,
    /// Net returns on the position.
    pub pnl: f64,
    /// Mark to market returns.
    pub m2m: f64,
    /// Unrealised intraday returns.
    pub unrealised: f64,
    /// Realised intraday returns.
    pub realised: f64,
    /// Quantity bought and added to the position.
    pub buy_quantity: i64,
    /// Average price at which quantities were bought.
    pub buy_price: f64,
    /// Net value of the bought quantities.
    pub buy_value: f64,
    /// Mark to market returns on the bought quantities.
    pub buy_m2m: f64,
    /// Quantity bought during the day.
    pub day_buy_quantity: i64,
    /// Average price of the quantities bought during the day.
    pub day_buy_price: f64,
    /// Net value of the quantities bought during the day.
    pub day_buy_value: f64,
    /// Quantity sold off from the position.
    pub sell_quantity: i64,
    /// Average price at which quantities were sold.
    pub sell_price: f64,
    /// Net value of the sold quantities.
    pub sell_value: f64,
    /// Mark to market returns on the sold quantities.
    pub sell_m2m: f64,
    /// Quantity sold during the day.
    pub day_sell_quantity: i64,
    /// Average price of the quantities sold during the day.
    pub day_sell_price: f64,
    /// Net value of the quantities sold during the day.
    pub day_sell_value: f64,
}

/// Whether a position to convert is an overnight or a day position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PositionType {
    /// Overnight position.
    #[serde(rename = "overnight")]
    Overnight,

    /// Day position.
    #[serde(rename = "day")]
    Day,
}

impl std::fmt::Display for PositionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Overnight => "overnight",
            Self::Day => "day",
        })
    }
}

/// A position conversion: `PUT /portfolio/positions`, form-encoded
/// (`kite-api-docs/docs/connect/v3/portfolio.md:463-497`).
///
/// A successful response reports the broker's result (`true`). It is not
/// reconciled position state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PositionConversionRequest {
    /// Tradingsymbol of the instrument.
    pub tradingsymbol: String,
    /// Exchange. `NONE` and `INDICES` are rejected.
    pub exchange: Exchange,
    /// BUY or SELL.
    pub transaction_type: TransactionType,
    /// Overnight or day.
    pub position_type: PositionType,
    /// Quantity to convert.
    pub quantity: crate::kite::protocol::Quantity,
    /// Existing margin product.
    pub old_product: ProductType,
    /// Margin product to convert to; must differ from `old_product`.
    pub new_product: ProductType,
}

impl PositionConversionRequest {
    /// Check the documented fields.
    pub fn validate(&self) -> Result<(), crate::kite::connect::models::RequestError> {
        use crate::kite::connect::models::RequestError;
        let err = |field, reason| Err(RequestError { field, reason });
        if !self.exchange.is_tradable() {
            return err("exchange", "is not a tradable exchange");
        }
        if self.tradingsymbol.is_empty() || self.tradingsymbol.len() > 64 {
            return err("tradingsymbol", "must be 1-64 bytes");
        }
        if self.old_product == self.new_product {
            return err("new_product", "must differ from old_product");
        }
        Ok(())
    }

    /// Form fields in the documented order.
    #[cfg_attr(not(feature = "http"), allow(dead_code))]
    pub(crate) fn form_pairs(&self) -> Vec<(&'static str, String)> {
        vec![
            ("tradingsymbol", self.tradingsymbol.clone()),
            ("exchange", self.exchange.to_string()),
            ("transaction_type", self.transaction_type.to_string()),
            ("position_type", self.position_type.to_string()),
            ("quantity", self.quantity.to_string()),
            ("old_product", self.old_product.to_string()),
            ("new_product", self.new_product.to_string()),
        ]
    }
}
