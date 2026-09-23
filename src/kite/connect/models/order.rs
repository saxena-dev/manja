//! Order and trade response types.
//!
//! These are broker evidence types: what the order book, order history and
//! trade book endpoints returned at the time of the request
//! (`kite-api-docs/docs/connect/v3/orders.md`). They are not reconciled
//! account state.
//!
//! Timestamps are offset-free IST strings parsed with the crate's single
//! broker datetime parser into `DateTime<FixedOffset>` at +05:30; `null`
//! stays `None`. Status, variety, type, product, validity and exchange
//! strings are [`Inbound`] values, so an undocumented value (the official
//! `order_info.json` fixture contains a `MODIFIED` status) is preserved
//! instead of failing the whole record or being coerced.
//!
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::kite::connect::models::exchange::Exchange;
use crate::kite::connect::models::order_enums::{
    OrderStatus, OrderType, OrderValidity, OrderVariety, ProductType, TransactionType,
};
use crate::kite::protocol::datetime::serde_opt_datetime;
use crate::kite::protocol::{BrokerTimestamp, Inbound, InstrumentToken};

/// The acknowledgement of a placement, modification or cancellation.
///
/// It identifies the order the OMS registered the request against. It is
/// **not** a fill, not confirmation that a modification took effect, and not
/// confirmation of cancellation: "Successful placement of an order via the
/// API does not imply its successful execution"
/// (`kite-api-docs/docs/connect/v3/orders.md:50-52`). The order's state is
/// learned from the order book, order history or order updates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderReceipt {
    /// The order ID the request was registered against.
    pub order_id: String,
}

/// An order as reported by the order book or order history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Order {
    /// Unique order ID.
    pub order_id: String,

    /// Order ID of the parent order (only applicable in case of multi-legged
    /// orders like CO).
    #[serde(default)]
    pub parent_order_id: Option<String>,

    /// Exchange generated order ID. Orders that don't reach the exchange have
    /// null IDs.
    #[serde(default)]
    pub exchange_order_id: Option<String>,

    /// Whether the order has been modified since placement by the user.
    #[serde(default)]
    pub modified: Option<bool>,

    /// ID of the user that placed the order. This may differ from the user's
    /// ID for orders placed outside of Kite, for instance by dealers.
    pub placed_by: String,

    /// Order variety.
    pub variety: Inbound<OrderVariety>,

    /// Current status of the order.
    pub status: Inbound<OrderStatus>,

    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,

    /// Exchange.
    pub exchange: Inbound<Exchange>,

    /// Instrument token.
    pub instrument_token: InstrumentToken,

    /// BUY or SELL.
    pub transaction_type: Inbound<TransactionType>,

    /// Order type.
    pub order_type: Inbound<OrderType>,

    /// Margin product.
    pub product: Inbound<ProductType>,

    /// Order validity.
    pub validity: Inbound<OrderValidity>,

    /// Order life span in minutes, for TTL validity orders.
    #[serde(default)]
    pub validity_ttl: Option<u32>,

    /// Price at which the order was placed (LIMIT orders).
    pub price: f64,

    /// Quantity ordered.
    pub quantity: u32,

    /// Trigger price (for SL, SL-M, CO orders).
    pub trigger_price: f64,

    /// Average price at which the order was executed (only for COMPLETE
    /// orders).
    pub average_price: f64,

    /// Pending quantity to be filled.
    pub pending_quantity: u32,

    /// Quantity that's been filled.
    pub filled_quantity: u32,

    /// Quantity disclosed to the public exchange order book.
    pub disclosed_quantity: u32,

    /// Quantity that's cancelled.
    pub cancelled_quantity: u32,

    /// Market protection setting.
    #[serde(default)]
    pub market_protection: Option<f64>,

    /// When the API registered the order (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub order_timestamp: Option<DateTime<FixedOffset>>,

    /// When the exchange registered the order (IST); `None` for orders that
    /// did not reach the exchange.
    #[serde(default, with = "serde_opt_datetime")]
    pub exchange_timestamp: Option<DateTime<FixedOffset>>,

    /// When the order's state last changed at the exchange (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub exchange_update_timestamp: Option<DateTime<FixedOffset>>,

    /// Textual description of the order's status.
    #[serde(default)]
    pub status_message: Option<String>,

    /// Raw textual description of the status, as received from the OMS.
    #[serde(default)]
    pub status_message_raw: Option<String>,

    /// A unique identifier for a particular auction.
    #[serde(default)]
    pub auction_number: Option<String>,

    /// Arbitrary fields that the system may attach to an order.
    #[serde(default)]
    pub meta: Option<serde_json::Value>,

    /// Optional order tag.
    #[serde(default)]
    pub tag: Option<String>,

    /// Order tags.
    #[serde(default)]
    pub tags: Option<Vec<String>>,

    /// Request GUID.
    #[serde(default)]
    pub guid: Option<String>,

    /// Total number of legs, for iceberg orders.
    #[serde(default)]
    pub iceberg_legs: Option<u32>,

    /// Split quantity of each iceberg leg.
    #[serde(default)]
    pub iceberg_quantity: Option<u32>,
}

/// A trade: one execution that filled all or part of an order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    /// Exchange generated trade ID.
    pub trade_id: String,

    /// Unique order ID.
    pub order_id: String,

    /// Exchange generated order ID.
    #[serde(default)]
    pub exchange_order_id: Option<String>,

    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,

    /// Exchange.
    pub exchange: Inbound<Exchange>,

    /// Instrument token. The official fixtures carry it as a number.
    pub instrument_token: InstrumentToken,

    /// BUY or SELL.
    pub transaction_type: Inbound<TransactionType>,

    /// Margin product.
    pub product: Inbound<ProductType>,

    /// Price at which the quantity was filled.
    pub average_price: f64,

    /// Filled quantity.
    pub quantity: i64,

    /// When the trade was filled at the exchange (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub fill_timestamp: Option<DateTime<FixedOffset>>,

    /// When the API registered the order. The trade book reports a bare
    /// time of day here (official `trades.json`), which is kept as such.
    #[serde(default)]
    pub order_timestamp: Option<BrokerTimestamp>,

    /// When the exchange registered the order (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub exchange_timestamp: Option<DateTime<FixedOffset>>,
}
