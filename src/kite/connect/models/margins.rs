//! Margin and charges calculation types
//! (`kite:margins.md`).
//!
//! These endpoints take JSON bodies, not form fields: "Requests to the above
//! endpoints are JSON POST and it needs `application/json` header"
//! (`kite:margins.md:13`). Order margins take and return arrays
//! (`kite:margins.md:15-85`); basket margins return initial, final and per-order
//! margins (`kite:margins.md:135-344`); the virtual contract note returns
//! order-wise charges (`kite:margins.md:345-500`).
//!
//! Results are broker calculations at the time of the request, not reserved
//! or blocked margin.
//!
use serde::{Deserialize, Serialize};

use crate::kite::connect::models::exchange::Exchange;
use crate::kite::connect::models::order::RequestError;
use crate::kite::connect::models::{OrderType, OrderVariety, ProductType, TransactionType};
use crate::kite::protocol::{Inbound, Quantity};

fn invalid(field: &'static str, reason: &'static str) -> Result<(), RequestError> {
    Err(RequestError { field, reason })
}

fn check_order_shape(exchange: &Exchange, tradingsymbol: &str) -> Result<(), RequestError> {
    if !exchange.is_tradable() {
        return invalid("exchange", "is not a tradable exchange");
    }
    if tradingsymbol.is_empty() || tradingsymbol.len() > 64 {
        return invalid("tradingsymbol", "must be 1-64 bytes");
    }
    Ok(())
}

fn check_non_negative(field: &'static str, v: f64) -> Result<(), RequestError> {
    if !v.is_finite() || v < 0.0 {
        return invalid(field, "must be finite and not negative");
    }
    Ok(())
}

/// One order in an order-margin or basket-margin calculation
/// (`kite:margins.md:87-98`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderMarginRequest {
    /// Exchange. `NONE` and `INDICES` are rejected.
    pub exchange: Exchange,
    /// Exchange tradingsymbol.
    pub tradingsymbol: String,
    /// BUY or SELL.
    pub transaction_type: TransactionType,
    /// Order variety.
    pub variety: OrderVariety,
    /// Margin product.
    pub product: ProductType,
    /// Order type.
    pub order_type: OrderType,
    /// Quantity.
    pub quantity: Quantity,
    /// Price, for LIMIT orders; `0` otherwise, as in the documented example.
    pub price: f64,
    /// Trigger price, for SL, SL-M and CO orders; `0` otherwise.
    pub trigger_price: f64,
}

impl OrderMarginRequest {
    /// Check the documented fields.
    pub fn validate(&self) -> Result<(), RequestError> {
        check_order_shape(&self.exchange, &self.tradingsymbol)?;
        check_non_negative("price", self.price)?;
        check_non_negative("trigger_price", self.trigger_price)
    }
}

/// One order in a virtual contract note calculation (`kite:margins.md:391-403`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderChargesRequest {
    /// Order ID; for a hypothetical order it may be any string.
    pub order_id: String,
    /// Exchange. `NONE` and `INDICES` are rejected.
    pub exchange: Exchange,
    /// Exchange tradingsymbol.
    pub tradingsymbol: String,
    /// BUY or SELL.
    pub transaction_type: TransactionType,
    /// Order variety.
    pub variety: OrderVariety,
    /// Margin product.
    pub product: ProductType,
    /// Order type.
    pub order_type: OrderType,
    /// Quantity.
    pub quantity: Quantity,
    /// Average execution price; must be positive ("Should be non-zero").
    pub average_price: f64,
}

impl OrderChargesRequest {
    /// Check the documented fields.
    pub fn validate(&self) -> Result<(), RequestError> {
        if self.order_id.is_empty() || self.order_id.len() > 64 {
            return invalid("order_id", "must be 1-64 bytes");
        }
        check_order_shape(&self.exchange, &self.tradingsymbol)?;
        if !self.average_price.is_finite() || self.average_price <= 0.0 {
            return invalid("average_price", "must be finite and positive");
        }
        Ok(())
    }
}

/// Realised and unrealised profit and loss.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)] // Public name kept for compatibility.
pub struct PNL {
    /// Realised profit and loss.
    pub realised: f64,
    /// Unrealised profit and loss.
    pub unrealised: f64,
}

/// Goods and Services Tax components.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)] // Public name kept for compatibility.
pub struct GST {
    /// Integrated GST.
    pub igst: f64,
    /// Central GST.
    pub cgst: f64,
    /// State GST.
    pub sgst: f64,
    /// Total GST.
    pub total: f64,
}

/// The charges breakdown of an order (`kite:margins.md:119-133`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Charges {
    /// Tax levied for each transaction on the exchanges.
    pub transaction_tax: f64,
    /// Type of transaction tax; may be empty.
    pub transaction_tax_type: String,
    /// Charge levied by the exchange on the day's turnover.
    pub exchange_turnover_charge: f64,
    /// Charge levied by SEBI on the day's turnover.
    pub sebi_turnover_charge: f64,
    /// Brokerage.
    pub brokerage: f64,
    /// Stamp duty.
    pub stamp_duty: f64,
    /// GST.
    pub gst: GST,
    /// Total charges.
    pub total: f64,
}

/// The margins of one order, or an aggregate in a basket
/// (`kite:margins.md:100-117`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderMargin {
    /// `equity` or `commodity`; empty in basket aggregates.
    pub r#type: String,
    /// Trading symbol; empty in basket aggregates.
    pub tradingsymbol: String,
    /// Exchange; empty (`Exchange::NONE`) in basket aggregates.
    pub exchange: Inbound<Exchange>,
    /// SPAN margins.
    pub span: f64,
    /// Exposure margins.
    pub exposure: f64,
    /// Option premium.
    pub option_premium: f64,
    /// Additional margins.
    pub additional: f64,
    /// BO margins.
    pub bo: f64,
    /// Cash credit.
    pub cash: f64,
    /// VAR.
    pub var: f64,
    /// Realised and unrealised profit and loss.
    pub pnl: PNL,
    /// Margin leverage allowed for the trade.
    pub leverage: f64,
    /// Charges breakdown.
    pub charges: Charges,
    /// Total margin block.
    pub total: f64,
}

/// Basket margins (`kite:margins.md:135-344`).
///
/// The `charges` field can omit `transaction_tax` for baskets that mix
/// segments with different tax types; the per-order `orders` carry the
/// order-wise charges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BasketMargin {
    /// Total margins required to execute the orders.
    pub initial: OrderMargin,
    /// Total margins with the spread benefit.
    pub r#final: OrderMargin,
    /// Individual margins per order.
    pub orders: Vec<OrderMargin>,
    /// Final charges.
    pub charges: Charges,
}

/// The charges of one order in a virtual contract note
/// (`kite:margins.md:488-500`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderCharges {
    /// BUY or SELL.
    pub transaction_type: Inbound<TransactionType>,
    /// Exchange tradingsymbol.
    pub tradingsymbol: String,
    /// Exchange.
    pub exchange: Inbound<Exchange>,
    /// Order variety.
    pub variety: Inbound<OrderVariety>,
    /// Margin product.
    pub product: Inbound<ProductType>,
    /// Order type.
    pub order_type: Inbound<OrderType>,
    /// Quantity.
    pub quantity: i64,
    /// Price at which the order is completed.
    pub price: f64,
    /// Charges breakdown.
    pub charges: Charges,
}
