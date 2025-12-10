//! Order and trade related types.
//!
//! This module defines structures related to orders and trades. It includes
//! detailed representations of orders, trades, and their various attributes,
//! making it easier to manage and process trading activities.
//!
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use serde::{Deserialize, Deserializer, Serialize};

use crate::models::order_enums::{OrderStatus, OrderType, ProductType, TransactionType};

use super::order_enums::OrderVariety;

/// Parses a date-time string into a `DateTime<FixedOffset>` with the Indian Standard
/// Time (IST) offset (+05:30).
///
/// # Arguments
///
/// * `deserializer` - The deserializer to use for parsing the date-time string.
///
/// # Returns
///
/// A `Result` containing an optional `DateTime<FixedOffset>` or an error if the parsing fails.
///
fn parse_datetime<'de, D>(deserializer: D) -> Result<Option<DateTime<FixedOffset>>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<&str> = Option::deserialize(deserializer)?;
    if let Some(s) = s {
        let naive =
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").map_err(serde::de::Error::custom)?;
        let offset = FixedOffset::east_opt(5 * 3600 + 1800)
            .ok_or_else(|| serde::de::Error::custom("invalid fixed offset"))?;
        let datetime = offset
            .from_local_datetime(&naive)
            .single()
            .ok_or_else(|| serde::de::Error::custom("ambiguous local time"))?;
        Ok(Some(datetime))
    } else {
        Ok(None)
    }
}

fn deserialize_guid<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<String> = Option::deserialize(deserializer)?;
    Ok(value.unwrap_or_default())
}

/// Represents an order received (and acknowledged) by Zerodha's OMS.
///
#[derive(Debug, Serialize, Deserialize)]
pub struct OrderReceipt {
    /// When an order is successfully placed, the API returns an `order_id`.
    pub order_id: String,
}

/// Represents an order in the trading system.
///
/// This struct contains details about an order, including its status, timestamps,
/// and various parameters related to the order's execution.
#[derive(Debug, Serialize, Deserialize)]
pub struct Order {
    /// Unique order ID.
    ///
    /// Placing an order implies registering it with the OMS via the API. This
    /// does not guarantee the order's receipt at the exchange. The fate of an
    /// order is dependent on several factors including market hours, availability
    /// of funds, risk checks and so on. Under normal circumstances, order
    /// placement, receipt by the OMS, transport to the exchange, execution,
    /// and the confirmation roundtrip happen instantly.
    ///
    /// When an order is successfully placed, the API returns an `order_id`.
    pub order_id: String,

    /// Order ID of the parent order (only applicable in case of multi-legged
    /// orders like CO).
    pub parent_order_id: Option<String>,

    /// Exchange generated order ID. Orders that don't reach the exchange have null IDs.
    pub exchange_order_id: Option<String>,

    /// Indicates whether the order has been modified since placement by the user.
    pub modified: bool,

    /// ID of the user that placed the order. This may differ from the user's ID
    /// for orders placed outside of Kite, for instance, by dealers at the brokerage
    /// using dealer terminals.
    pub placed_by: String,

    /// Order variety (regular, amo, co, etc.).
    pub variety: OrderVariety,

    /// Current status of the order. Most common values are COMPLETE, REJECTED,
    /// CANCELLED, and OPEN. There may be other values as well.
    pub status: OrderStatus,

    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,

    /// Exchange where the order was placed.
    pub exchange: String,

    /// The numerical identifier issued by the exchange representing the instrument.
    /// Used for subscribing to live market data over WebSocket.
    pub instrument_token: u64,

    /// Transaction type (BUY or SELL).
    pub transaction_type: TransactionType,

    /// Order type (MARKET, LIMIT, etc.).
    pub order_type: OrderType,

    /// Margin product to use for the order (margins are blocked based on this).
    pub product: ProductType,

    /// Order validity.
    pub validity: String,

    /// Price at which the order was placed (LIMIT orders).
    pub price: f64,

    /// Quantity ordered.
    pub quantity: u32,

    /// Trigger price (for SL, SL-M, CO orders).
    pub trigger_price: f64,

    /// Average price at which the order was executed (only for COMPLETE orders).
    pub average_price: f64,

    /// Pending quantity to be filled.
    pub pending_quantity: u32,

    /// Quantity that's been filled.
    pub filled_quantity: u32,

    /// Quantity to be disclosed (may be different from actual quantity) to the
    /// public exchange orderbook. Only for equities.
    pub disclosed_quantity: u32,

    /// Timestamp at which the order was registered by the API.
    #[serde(default, deserialize_with = "parse_datetime")]
    pub order_timestamp: Option<DateTime<FixedOffset>>,

    /// Timestamp at which the order was registered by the exchange. Orders that
    /// don't reach the exchange have null timestamps.
    #[serde(default, deserialize_with = "parse_datetime")]
    pub exchange_timestamp: Option<DateTime<FixedOffset>>,

    /// Timestamp at which an order's state changed at the exchange.
    #[serde(default, deserialize_with = "parse_datetime")]
    pub exchange_update_timestamp: Option<DateTime<FixedOffset>>,

    /// Textual description of the order's status. Failed orders come with a
    /// human-readable explanation.
    pub status_message: Option<String>,

    /// Raw textual description of the failed order's status, as received from the OMS.
    pub status_message_raw: Option<String>,

    /// Quantity that's cancelled.
    pub cancelled_quantity: u32,

    /// A unique identifier for a particular auction.
    pub auction_number: Option<String>,

    /// Map of arbitrary fields that the system may attach to an order.
    #[serde(default)]
    pub meta: serde_json::Value,

    /// An optional tag to apply to an order to identify it (alphanumeric,
    /// max 20 chars).
    pub tag: Option<String>,

    /// Unusable request ID to avoid order duplication.
    #[serde(default, deserialize_with = "deserialize_guid")]
    pub guid: String,

    /// The total number of legs for iceberg orders.
    pub iceberg_legs: Option<u32>,

    /// The split quantity for each iceberg leg order.
    pub iceberg_quantity: Option<u32>,

    /// The order life span in minutes for TTL validity orders.
    pub validity_ttl: Option<u32>,

    /// A list of tags associated with the order.
    pub tags: Option<Vec<String>>,
}

/// Represents a trade executed at the exchange.
///
/// An order may be executed in multiple chunks at the exchange depending on
/// market conditions. Each individual execution that partially fills an order
/// is a trade. Thus, an order may have one or more trades.
///
#[derive(Debug, Serialize, Deserialize)]
pub struct Trade {
    /// Exchange generated trade ID.
    pub trade_id: String,

    /// Unique order ID.
    pub order_id: String,

    /// Exchange generated order ID.
    pub exchange_order_id: Option<String>,

    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,

    /// Exchange.
    pub exchange: String,

    /// The numerical identifier issued by the exchange representing the instrument.
    /// Used for subscribing to live market data over WebSocket.
    pub instrument_token: u64,

    /// BUY or SELL transaction type.
    pub transaction_type: TransactionType,

    /// Margin product to use for the order (margins are blocked based on this).
    pub product: String,

    /// Price at which the quantity was filled.
    pub average_price: f64,

    /// Filled quantity.
    pub quantity: i64,

    /// Timestamp at which the trade was filled at the exchange.
    pub fill_timestamp: String,

    /// Timestamp at which the order was registered by the API.
    pub order_timestamp: String,

    /// Timestamp at which the order was registered by the exchange.
    pub exchange_timestamp: String,
}
