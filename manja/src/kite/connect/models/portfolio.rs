//! Portfolio related types.
//!
//! This module defines various structures and enums related to the user's
//! portfolio holdings, positions, and related actions in a trading system.
//! It provides detailed representations of holdings, positions, auctions, and
//! position conversion requests, making it easier to manage and process
//! portfolio activities.
//!
use crate::kite::connect::models::{
    exchange::Exchange,
    order_enums::{ProductType, TransactionType},
};

use serde::{Deserialize, Serialize};

/// Represents a holding in the user's portfolio, containing long-term equity
/// delivery stocks.
///
/// Holdings contain the user's portfolio of long-term equity delivery stocks.
/// An instrument in a holdings portfolio remains there indefinitely until it
/// is sold, delisted, or changed by the exchanges. Instruments in the holdings
/// reside in the user's DEMAT account, as settled by exchanges and clearing
/// institutions.
///
#[derive(Debug, Serialize, Deserialize)]
pub struct Holding {
    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,

    /// Exchange.
    pub exchange: Exchange,

    /// Unique instrument identifier (used for WebSocket subscriptions).
    pub instrument_token: u32,

    /// The standard ISIN representing stocks listed on multiple exchanges.
    pub isin: String,

    /// Quantity on T+1 day after order execution. Stocks are usually delivered
    /// into DEMAT accounts on T+2.
    pub t1_quantity: i64,

    /// Quantity delivered to Demat.
    pub realised_quantity: i64,

    /// Net quantity (T+1 + realised).
    pub quantity: i64,

    /// Quantity sold from the net holding quantity.
    pub used_quantity: i64,

    /// Quantity authorized at the depository for sale.
    pub authorised_quantity: i64,

    /// Quantity carried forward overnight.
    pub opening_quantity: i64,

    /// Date on which the user can sell the required holding stock.
    pub authorised_date: String,

    /// The price of the instrument.
    pub price: f64,

    /// Average price at which the net holding quantity was acquired.
    pub average_price: f64,

    /// Last traded market price of the instrument.
    pub last_price: f64,

    /// Closing price of the instrument from the last trading day.
    pub close_price: f64,

    /// Net returns on the stock; Profit and loss.
    pub pnl: f64,

    /// Day's change in absolute value for the stock.
    pub day_change: f64,

    /// Day's change in percentage for the stock.
    pub day_change_percentage: f64,

    /// Margin product applied to the holding.
    pub product: String,

    /// Quantity used as collateral.
    pub collateral_quantity: i64,

    /// Type of collateral.
    pub collateral_type: Option<String>,

    /// Indicates whether the holding has any price discrepancy.
    pub discrepancy: bool,
}

/// Represents an auction currently being held, with details about the security
/// being auctioned.
///
/// This struct contains details about an auction such as the auction number,
/// the security being auctioned, the last price of the security, and the quantity
/// of the security being offered. Only the stocks that you hold in your demat
/// account will be shown in the auctions list.
///
#[derive(Debug, Serialize, Deserialize)]
pub struct Auction {
    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,

    /// Exchange.
    pub exchange: String,

    /// Unique instrument identifier (used for WebSocket subscriptions).
    pub instrument_token: u32,

    /// The standard ISIN representing stocks listed on multiple exchanges.
    pub isin: String,

    /// Margin product applied to the holding.
    pub product: String,

    /// The price of the instrument.
    pub price: f64,

    /// Net quantity (T+1 + realised).
    pub quantity: i64,

    /// Quantity on T+1 day after order execution. Stocks are usually delivered
    /// into DEMAT accounts on T+2.
    pub t1_quantity: i64,

    /// Quantity delivered to Demat.
    pub realised_quantity: i64,

    /// Quantity authorized at the depository for sale.
    pub authorised_quantity: i64,

    /// Date on which the user can sell the required holding stock.
    pub authorised_date: String,

    /// Quantity carried forward overnight.
    pub opening_quantity: i64,

    /// Quantity used as collateral.
    pub collateral_quantity: i64,

    /// Type of collateral.
    pub collateral_type: Option<String>,

    /// Indicates whether the holding has any price discrepancy.
    pub discrepancy: bool,

    /// Average price at which the net holding quantity was acquired.
    pub average_price: f64,

    /// Last traded market price of the instrument.
    pub last_price: f64,

    /// Closing price of the instrument from the last trading day.
    pub close_price: f64,

    /// Net returns on the stock; Profit and loss.
    pub pnl: f64,

    /// Day's change in absolute value for the stock.
    pub day_change: f64,

    /// Day's change in percentage for the stock.
    pub day_change_percentage: f64,

    /// A unique identifier for a particular auction.
    pub auction_number: String,
}

/// Represents a position in the user's portfolio, containing short to medium-term
/// derivatives and intraday equity stocks.
///
/// Positions contain the user's portfolio of short to medium-term derivatives
/// (futures and options contracts) and intraday equity stocks.
/// Instruments in the positions portfolio remain there until they're sold or
/// until expiry, which, for derivatives, is typically three months.
/// Equity positions carried overnight move to the holdings portfolio the next
/// day.
///
/// The positions API returns two sets of positions, net and day.
/// Net is the actual, current net position portfolio, while day is a snapshot
/// of the buying and selling activity for that particular day.
///
#[derive(Debug, Serialize, Deserialize)]
pub struct Position {
    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,

    /// Exchange.
    pub exchange: String,

    /// The numerical identifier issued by the exchange representing the instrument.
    /// Used for subscribing to live market data over WebSocket.
    pub instrument_token: u32,

    /// Margin product applied to the position.
    pub product: String,

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

    /// Net returns on the position; Profit and loss.
    pub pnl: f64,

    /// Mark to market returns (computed based on the last close and the last
    /// traded price).
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

    /// Quantity bought and added to the position during the day.
    pub day_buy_quantity: i64,

    /// Average price at which quantities were bought during the day.
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

    /// Quantity sold off from the position during the day.
    pub day_sell_quantity: i64,

    /// Average price at which quantities were sold during the day.
    pub day_sell_price: f64,

    /// Net value of the quantities sold during the day.
    pub day_sell_value: f64,
}

/// Represents the variety of an order, either overnight or day positions.
///
/// This enum contains constant values used for placing different types of
/// orders.
///
#[derive(Debug, Serialize, Deserialize)]
pub enum PositionType {
    /// Overnight position.
    #[serde(rename = "overnight")]
    Overnight,

    /// Day position
    #[serde(rename = "day")]
    Day,
}

/// Represents the request parameters for a position conversion.
///
/// These parameters are required to convert a position's margin product.
///
#[derive(Debug, Serialize, Deserialize)]
pub struct PositionConversionRequest {
    /// Tradingsymbol of the instrument.
    pub tradingsymbol: String,

    /// Name of the exchange.
    pub exchange: Exchange,

    /// Transaction type: BUY or SELL.
    pub transaction_type: TransactionType,

    /// Position type: overnight or day.
    pub position_type: PositionType,

    /// Quantity to convert.
    pub quantity: i32,

    /// Existing margin product of the position.
    pub old_product: ProductType,

    /// Margin product to convert to.
    pub new_product: ProductType,
}
