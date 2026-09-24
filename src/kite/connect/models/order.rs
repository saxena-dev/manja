//! Order and trade response types.
//!
//! These are broker evidence types: what the order book, order history and
//! trade book endpoints returned at the time of the request
//! (`kite:orders.md`).
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
use crate::kite::protocol::{BrokerTimestamp, Inbound, InstrumentToken, OrderId};

/// The acknowledgement of a placement, modification or cancellation.
///
/// It identifies the order the OMS registered the request against. It is
/// **not** a fill, not confirmation that a modification took effect, and not
/// confirmation of cancellation: "Successful placement of an order via the
/// API does not imply its successful execution"
/// (`kite:orders.md:50-52`). The order's state is
/// learned from the order book, order history or order updates.
///
/// An automatically sliced placement (`autoslice = true` above the freeze
/// quantity, `kite:orders.md:548`) carries one result per further slice in
/// [`Self::slices`]. A slice can fail while others are placed, so check
/// every entry: a receipt with a [`SliceResult::Failed`] slice is **not** a
/// complete placement. The official mock (`autoslice_response.json`)
/// returns `{order_id, children}` while the documentation shows an array
/// whose first entry is that same order; both decode to the same receipt.
/// A slice with neither an order ID nor an error is a decode error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OrderReceipt {
    /// The order ID the request was registered against: the first slice
    /// of a sliced placement.
    pub order_id: OrderId,
    /// The further slices of an automatically sliced placement, in broker
    /// order; empty otherwise.
    #[serde(rename = "children", skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<SliceResult>,
}

impl OrderReceipt {
    /// Whether any slice failed.
    pub fn has_failed_slices(&self) -> bool {
        self.slices
            .iter()
            .any(|s| matches!(s, SliceResult::Failed(_)))
    }
}

/// The outcome of one slice of an automatically sliced placement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum SliceResult {
    /// The slice was registered as its own order.
    Placed {
        /// Its order ID.
        order_id: OrderId,
    },
    /// The slice was rejected.
    Failed(SliceError),
}

/// Why one slice was rejected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SliceError {
    /// HTTP-style code, if given.
    #[serde(default)]
    pub code: Option<u16>,
    /// Broker error type, preserved if unknown.
    #[serde(default)]
    pub error_type: Option<Inbound<crate::kite::error::KiteApiException>>,
    /// Broker message.
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Deserialize)]
struct SliceWire {
    #[serde(default)]
    order_id: Option<OrderId>,
    #[serde(default)]
    error: Option<SliceError>,
}

impl SliceWire {
    fn resolve<E: serde::de::Error>(self) -> Result<SliceResult, E> {
        match (self.order_id, self.error) {
            (Some(order_id), None) => Ok(SliceResult::Placed { order_id }),
            (None, Some(e)) => Ok(SliceResult::Failed(e)),
            _ => Err(E::custom("a slice needs exactly one of order_id and error")),
        }
    }
}

impl<'de> Deserialize<'de> for OrderReceipt {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Shape {
            Object {
                order_id: OrderId,
                #[serde(default)]
                children: Vec<SliceWire>,
            },
            Array(Vec<SliceWire>),
        }
        let (order_id, rest) = match Shape::deserialize(d)? {
            Shape::Object { order_id, children } => (order_id, children),
            Shape::Array(mut all) => {
                if all.is_empty() {
                    return Err(serde::de::Error::custom("an empty slice list"));
                }
                let first = all.remove(0);
                match first.resolve::<D::Error>()? {
                    SliceResult::Placed { order_id } => (order_id, all),
                    _ => {
                        return Err(serde::de::Error::custom(
                            "the first slice of a placement was not placed",
                        ))
                    }
                }
            }
        };
        let slices = rest
            .into_iter()
            .map(SliceWire::resolve::<D::Error>)
            .collect::<Result<_, _>>()?;
        Ok(Self { order_id, slices })
    }
}

/// An order as reported by the order book or order history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Order {
    /// Unique order ID.
    pub order_id: OrderId,

    /// Order ID of the parent order (only applicable in case of multi-legged
    /// orders like CO).
    #[serde(default)]
    pub parent_order_id: Option<OrderId>,

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
    pub order_id: OrderId,

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

// --- [ Request DTOs ] ---

/// Why a request was rejected before admission. Nothing was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RequestError {
    /// The offending field.
    pub field: &'static str,
    /// Why it was rejected.
    pub reason: &'static str,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid `{}`: {}", self.field, self.reason)
    }
}

impl std::error::Error for RequestError {}

pub(super) fn invalid(field: &'static str, reason: &'static str) -> Result<(), RequestError> {
    Err(RequestError { field, reason })
}

fn check_price(field: &'static str, p: Option<f64>) -> Result<(), RequestError> {
    match p {
        Some(v) if !v.is_finite() || v <= 0.0 => invalid(field, "must be finite and positive"),
        _ => Ok(()),
    }
}

// kite:orders.md:37,101: greater than 0 and up to 100, or -1 for automatic.
fn check_market_protection(p: Option<f64>) -> Result<(), RequestError> {
    match p {
        Some(v) if v == -1.0 || (v > 0.0 && v <= 100.0) => Ok(()),
        Some(_) => invalid("market_protection", "must be -1 or in (0, 100]"),
        None => Ok(()),
    }
}

#[cfg_attr(not(feature = "http"), allow(dead_code))]
fn push<T: std::fmt::Display>(
    pairs: &mut Vec<(&'static str, String)>,
    k: &'static str,
    v: Option<T>,
) {
    if let Some(v) = v {
        pairs.push((k, v.to_string()));
    }
}

/// A new order: `POST /orders/{variety}`, form-encoded
/// (`kite:orders.md:58-103`).
///
/// It has no order ID, status, fill or timestamp: those are response-only
/// facts, and a request type that cannot hold them cannot fabricate them:
///
/// ```compile_fail
/// # use manja::kite::connect::models::*;
/// # use manja::kite::protocol::Quantity;
/// let mut req = PlaceOrderRequest::new(
///     OrderVariety::Regular, Exchange::NSE, "INFY", TransactionType::BUY,
///     OrderType::Market, Quantity::new(1).unwrap(), ProductType::CashAndCarry,
/// );
/// req.order_id = "151220000000000".into();
/// ```
///
/// [`Self::validate`] checks the protocol shape only: the route, identifiers,
/// quantities and order-type field combinations. Nothing that depends on the
/// account, such as funds or margins, is checked; the broker does that.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceOrderRequest {
    /// Order variety; selects the route.
    pub variety: OrderVariety,
    /// Exchange. `NONE` and `INDICES` are rejected.
    pub exchange: Exchange,
    /// Exchange tradingsymbol.
    pub tradingsymbol: String,
    /// BUY or SELL.
    pub transaction_type: TransactionType,
    /// Order type.
    pub order_type: OrderType,
    /// Quantity to transact.
    pub quantity: crate::kite::protocol::Quantity,
    /// Margin product.
    pub product: ProductType,
    /// Order validity; `DAY` by default.
    pub validity: OrderValidity,
    /// Price: required for LIMIT and SL, forbidden for MARKET and SL-M.
    pub price: Option<f64>,
    /// Trigger price: required for SL and SL-M, forbidden otherwise.
    pub trigger_price: Option<f64>,
    /// Quantity to disclose publicly; at most `quantity`.
    pub disclosed_quantity: Option<u32>,
    /// Life span in minutes: required with TTL validity, forbidden otherwise.
    pub validity_ttl: Option<u32>,
    /// Iceberg legs, 2 to 50: required for the iceberg variety only.
    pub iceberg_legs: Option<u32>,
    /// Iceberg leg quantity: required for the iceberg variety only.
    pub iceberg_quantity: Option<u32>,
    /// Auction number: required for the auction variety only.
    pub auction_number: Option<String>,
    /// Market protection, -1 or in (0, 100]: MARKET and SL-M only.
    pub market_protection: Option<f64>,
    /// Automatic slicing above freeze quantity.
    pub autoslice: Option<bool>,
    /// Optional tag, at most 20 ASCII letters and digits.
    pub tag: Option<String>,
}

impl PlaceOrderRequest {
    /// A request with `DAY` validity and no optional fields.
    pub fn new(
        variety: OrderVariety,
        exchange: Exchange,
        tradingsymbol: impl Into<String>,
        transaction_type: TransactionType,
        order_type: OrderType,
        quantity: crate::kite::protocol::Quantity,
        product: ProductType,
    ) -> Self {
        Self {
            variety,
            exchange,
            tradingsymbol: tradingsymbol.into(),
            transaction_type,
            order_type,
            quantity,
            product,
            validity: OrderValidity::Day,
            price: None,
            trigger_price: None,
            disclosed_quantity: None,
            validity_ttl: None,
            iceberg_legs: None,
            iceberg_quantity: None,
            auction_number: None,
            market_protection: None,
            autoslice: None,
            tag: None,
        }
    }

    /// Check the documented field combinations.
    pub fn validate(&self) -> Result<(), RequestError> {
        if !self.exchange.is_tradable() {
            return invalid("exchange", "is not a tradable exchange");
        }
        if self.tradingsymbol.is_empty() || self.tradingsymbol.len() > 64 {
            return invalid("tradingsymbol", "must be 1-64 bytes");
        }
        check_price("price", self.price)?;
        check_price("trigger_price", self.trigger_price)?;
        match self.order_type {
            OrderType::Limit if self.price.is_none() => {
                return invalid("price", "is required for LIMIT orders")
            }
            OrderType::Stoploss if self.price.is_none() || self.trigger_price.is_none() => {
                return invalid("trigger_price", "SL orders need price and trigger_price")
            }
            OrderType::StoplossMarket if self.trigger_price.is_none() => {
                return invalid("trigger_price", "is required for SL-M orders")
            }
            OrderType::Market | OrderType::StoplossMarket if self.price.is_some() => {
                return invalid("price", "is not accepted for MARKET or SL-M orders")
            }
            OrderType::Market | OrderType::Limit
                if self.trigger_price.is_some() && self.variety != OrderVariety::Cover =>
            {
                return invalid("trigger_price", "is only for SL, SL-M and cover orders")
            }
            _ => {}
        }
        if self.market_protection.is_some()
            && !matches!(
                self.order_type,
                OrderType::Market | OrderType::StoplossMarket
            )
        {
            return invalid("market_protection", "is only for MARKET and SL-M orders");
        }
        check_market_protection(self.market_protection)?;
        if self
            .disclosed_quantity
            .is_some_and(|d| d > self.quantity.get())
        {
            return invalid("disclosed_quantity", "exceeds quantity");
        }
        match (self.validity, self.validity_ttl) {
            (OrderValidity::TimeToLive, None | Some(0)) => {
                return invalid(
                    "validity_ttl",
                    "a positive TTL is required with TTL validity",
                )
            }
            (OrderValidity::Day | OrderValidity::ImmediateOrCancel, Some(_)) => {
                return invalid("validity_ttl", "is only for TTL validity")
            }
            _ => {}
        }
        let iceberg = self.variety == OrderVariety::Iceberg;
        match (iceberg, self.iceberg_legs, self.iceberg_quantity) {
            (true, Some(legs), Some(q)) if (2..=50).contains(&legs) && q > 0 => {}
            (true, _, _) => {
                return invalid(
                    "iceberg_legs",
                    "iceberg orders need 2-50 legs and a leg quantity",
                )
            }
            (false, None, None) => {}
            (false, _, _) => return invalid("iceberg_legs", "is only for iceberg orders"),
        }
        match (self.variety == OrderVariety::Auction, &self.auction_number) {
            (true, Some(n)) if !n.is_empty() => {}
            (true, _) => return invalid("auction_number", "is required for auction orders"),
            (false, None) => {}
            (false, Some(_)) => return invalid("auction_number", "is only for auction orders"),
        }
        if let Some(tag) = &self.tag
            && (tag.is_empty() || tag.len() > 20 || !tag.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return invalid("tag", "must be 1-20 ASCII letters or digits");
        }
        Ok(())
    }

    /// Form fields in a fixed order.
    #[cfg_attr(not(feature = "http"), allow(dead_code))]
    pub(crate) fn form_pairs(&self) -> Vec<(&'static str, String)> {
        let mut p = vec![
            ("tradingsymbol", self.tradingsymbol.clone()),
            ("exchange", self.exchange.to_string()),
            ("transaction_type", self.transaction_type.to_string()),
            ("order_type", self.order_type.to_string()),
            ("quantity", self.quantity.to_string()),
            ("product", self.product.to_string()),
            ("validity", self.validity.to_string()),
        ];
        push(&mut p, "price", self.price);
        push(&mut p, "trigger_price", self.trigger_price);
        push(&mut p, "disclosed_quantity", self.disclosed_quantity);
        push(&mut p, "validity_ttl", self.validity_ttl);
        push(&mut p, "iceberg_legs", self.iceberg_legs);
        push(&mut p, "iceberg_quantity", self.iceberg_quantity);
        push(&mut p, "auction_number", self.auction_number.as_ref());
        push(&mut p, "market_protection", self.market_protection);
        push(&mut p, "autoslice", self.autoslice);
        push(&mut p, "tag", self.tag.as_ref());
        p
    }
}

/// Changes to an open or pending order: `PUT /orders/{variety}/{order_id}`,
/// form-encoded (`kite:orders.md:105-146`).
///
/// Only the fields that are set are sent. A successful modification returns
/// an [`OrderReceipt`], which does not prove the modification took effect.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModifyOrderRequest {
    /// New order type.
    pub order_type: Option<OrderType>,
    /// New quantity.
    pub quantity: Option<crate::kite::protocol::Quantity>,
    /// New price.
    pub price: Option<f64>,
    /// New trigger price.
    pub trigger_price: Option<f64>,
    /// New disclosed quantity.
    pub disclosed_quantity: Option<u32>,
    /// New validity.
    pub validity: Option<OrderValidity>,
    /// New TTL in minutes; only with TTL validity.
    pub validity_ttl: Option<u32>,
    /// Market protection, -1 or in (0, 100].
    pub market_protection: Option<f64>,
}

impl ModifyOrderRequest {
    /// Check the documented fields for `variety`: cover orders accept only
    /// price and trigger price, and at least one field must be set.
    pub fn validate(&self, variety: OrderVariety) -> Result<(), RequestError> {
        let any = self.order_type.is_some()
            || self.quantity.is_some()
            || self.price.is_some()
            || self.trigger_price.is_some()
            || self.disclosed_quantity.is_some()
            || self.validity.is_some()
            || self.validity_ttl.is_some()
            || self.market_protection.is_some();
        if !any {
            return invalid("request", "sets no field to modify");
        }
        check_price("price", self.price)?;
        check_price("trigger_price", self.trigger_price)?;
        check_market_protection(self.market_protection)?;
        if variety == OrderVariety::Cover
            && (self.order_type.is_some()
                || self.quantity.is_some()
                || self.disclosed_quantity.is_some()
                || self.validity.is_some()
                || self.validity_ttl.is_some()
                || self.market_protection.is_some())
        {
            return invalid(
                "variety",
                "cover orders accept only price and trigger_price",
            );
        }
        if self.validity_ttl.is_some() && self.validity != Some(OrderValidity::TimeToLive) {
            return invalid("validity_ttl", "is only for TTL validity");
        }
        if self.validity == Some(OrderValidity::TimeToLive) && self.validity_ttl.unwrap_or(0) == 0 {
            return invalid(
                "validity_ttl",
                "a positive TTL is required with TTL validity",
            );
        }
        if let (Some(d), Some(q)) = (self.disclosed_quantity, self.quantity)
            && d > q.get()
        {
            return invalid("disclosed_quantity", "exceeds quantity");
        }
        Ok(())
    }

    /// Form fields in a fixed order.
    #[cfg_attr(not(feature = "http"), allow(dead_code))]
    pub(crate) fn form_pairs(&self) -> Vec<(&'static str, String)> {
        let mut p = Vec::new();
        push(&mut p, "order_type", self.order_type);
        push(&mut p, "quantity", self.quantity);
        push(&mut p, "price", self.price);
        push(&mut p, "trigger_price", self.trigger_price);
        push(&mut p, "disclosed_quantity", self.disclosed_quantity);
        push(&mut p, "validity", self.validity);
        push(&mut p, "validity_ttl", self.validity_ttl);
        push(&mut p, "market_protection", self.market_protection);
        p
    }
}
