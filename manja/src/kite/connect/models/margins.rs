//! Types for calculating margins and charges on orders.
//!
//! This module defines structures and types related to calculating margins, charges,
//! and profit and loss for orders. It includes requests and responses for order
//! margins and charges, along with detailed structures for GST and other applicable
//! charges.
//!
use crate::kite::connect::models::exchange::Exchange;
use crate::kite::connect::models::{OrderType, OrderVariety, ProductType, TransactionType};

use serde::{Deserialize, Serialize};

/// Represents a request for calculating margins for an order.
///
/// This structure contains all necessary information to request margin calculations
/// for a specific order, including exchange, transaction type, order type, and more.
///
#[derive(Serialize, Deserialize, Debug)]
pub struct OrderMarginRequest {
    /// Name of the exchange
    pub exchange: Exchange,
    /// Exchange tradingsymbol of the instrument
    pub tradingsymbol: String,
    /// Type of transaction (BUY/SELL)
    pub transaction_type: TransactionType,
    /// Order variety (regular, amo, co etc.)
    pub variety: OrderVariety,
    /// Margin product to use for the order (margins are blocked based on this)
    pub product: ProductType,
    /// Order type (MARKET, LIMIT etc.)
    pub order_type: OrderType,
    /// Quantity of the order
    pub quantity: i64,
    /// Price at which the order is going to be placed (for LIMIT orders)
    pub price: f64,
    /// Trigger price (for SL, SL-M, CO orders)
    pub trigger_price: f64,
}

/// Represents the profit and loss (PNL) structure.
///
/// This structure holds the realised and unrealised profit and loss values.
///
#[derive(Serialize, Deserialize, Debug)]
pub struct PNL {
    /// Realised profit and loss
    pub realised: f64,
    /// Unrealised profit and loss
    pub unrealised: f64,
}

/// Represents the GST structure.
///
/// This structure holds details about various GST components like IGST, CGST, and SGST.
///
#[derive(Serialize, Deserialize, Debug)]
pub struct GST {
    /// Integrated Goods and Services Tax
    pub igst: f64,
    /// Central Goods and Services Tax
    pub cgst: f64,
    /// State Goods and Services Tax
    pub sgst: f64,
    /// Total GST
    pub total: f64,
}

/// Represents the various charges applied to an order.
///
/// This structure includes transaction taxes, turnover charges, brokerage, stamp duty, and GST.
///
#[derive(Serialize, Deserialize, Debug)]
pub struct Charges {
    /// Tax levied for each transaction on the exchanges
    pub transaction_tax: f64,
    /// Type of transaction tax
    pub transaction_tax_type: String,
    /// Charge levied by the exchange on the total turnover of the day
    pub exchange_turnover_charge: f64,
    /// Charge levied by SEBI on the total turnover of the day
    pub sebi_turnover_charge: f64,
    /// Brokerage charge for a particular trade
    pub brokerage: f64,
    /// Duty levied on the transaction value by Government of India
    pub stamp_duty: f64,
    /// GST structure
    pub gst: GST,
    /// Total charges
    pub total: f64,
}

/// Represents the margin details for an order.
///
/// This structure provides detailed information about the margins required for
/// an order, including SPAN margins, exposure margins, option premiums, and more.
///
#[derive(Serialize, Deserialize, Debug)]
pub struct OrderMargin {
    /// Type of order (equity/commodity)
    pub r#type: String,
    /// Trading symbol of the instrument
    pub tradingsymbol: String,
    /// Name of the exchange
    #[serde(default)]
    pub exchange: Exchange,
    /// SPAN margins
    pub span: f64,
    /// Exposure margins
    pub exposure: f64,
    /// Option premium
    pub option_premium: f64,
    /// Additional margins
    pub additional: f64,
    /// BO margins
    pub bo: f64,
    /// Cash credit
    pub cash: f64,
    /// VAR
    pub var: f64,
    /// Realised and unrealised profit and loss
    pub pnl: PNL,
    /// Margin leverage allowed for the trade
    pub leverage: i64,
    /// The breakdown of the various charges that will be applied to an order
    pub charges: Charges,
    /// Total margin block
    pub total: f64,
}

/// Represents the margin details for a basket of orders.
///
/// This structure provides an aggregated view of margins required for executing
/// a basket of orders, along with individual order margins and final charges.
///
/// Note: The [charges] field can be ignored as it may not include `transaction_tax`
/// charges because baskets can contain both `mcx` and `equity` instruments,
/// with different tax types (STT or CTT). Users can refer to the individual
/// order charges response in the [orders] field.
///
#[derive(Serialize, Deserialize, Debug)]
pub struct BasketMargin {
    /// Total margins required to execute the orders
    pub initial: OrderMargin,
    /// Total margins with the spread benefit
    pub r#final: OrderMargin,
    /// Individual margins per order
    pub orders: Vec<OrderMargin>,
    /// Final charges
    pub charges: Charges,
}

/// Represents a request for calculating charges for an order.
///
/// This structure contains all necessary information to request charge calculations
/// for a specific order, including exchange, transaction type, order type, and more.
///
#[derive(Serialize, Deserialize, Debug)]
pub struct OrderChargesRequest {
    /// Unique order ID (It can be any random string to calculate charges for an imaginary order)
    pub order_id: String,
    /// Name of the exchange
    pub exchange: Exchange,
    /// Exchange tradingsymbol of the instrument
    pub tradingsymbol: String,
    /// Type of transaction (BUY/SELL)
    pub transaction_type: TransactionType,
    /// Order variety (regular, amo, co etc.)
    pub variety: OrderVariety,
    /// Margin product to use for the order (margins are blocked based on this)
    pub product: ProductType,
    /// Order type (MARKET, LIMIT etc.)
    pub order_type: OrderType,
    /// Quantity of the order
    pub quantity: i64,
    /// Average price at which the order was executed (Note: Should be non-zero)
    pub average_price: f64,
}

/// Represents the detailed charges for an order.
///
/// This structure provides a breakdown of all the charges that will be applied
/// to an order, including transaction tax, exchange turnover charge, SEBI turnover
/// charge, brokerage, and GST.
///
#[derive(Serialize, Deserialize, Debug)]
pub struct OrderCharges {
    /// Type of transaction being processed (BUY/SELL).
    pub transaction_type: String,
    /// Exchange `tradingsymbol` of the instrument
    pub tradingsymbol: String,
    /// Name of the exchange
    pub exchange: Exchange,
    /// Order variety (regular, amo, co etc.)
    pub variety: OrderVariety,
    /// Margin product to use for the order (margins are blocked based on this)
    pub product: ProductType,
    /// Order type (MARKET, LIMIT etc.)
    pub order_type: OrderType,
    /// Quantity of the order
    pub quantity: i64,
    /// Price at which the order is completed
    pub price: f64,
    /// The breakdown of the various charges that will be applied to an order
    pub charges: Charges,
}
