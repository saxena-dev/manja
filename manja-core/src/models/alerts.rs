use serde::{Deserialize, Serialize};

use crate::models::{exchange::Exchange, OrderType, ProductType, TransactionType};

/// Type of alert.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum AlertType {
    #[serde(rename = "simple")]
    Simple,
    #[serde(rename = "ato")]
    Ato,
}

/// Status of an alert.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum AlertStatus {
    #[serde(rename = "enabled")]
    Enabled,
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "deleted")]
    Deleted,
}

/// Comparison operator used in alert conditions.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum AlertOperator {
    #[serde(rename = "<")]
    LessThan,
    #[serde(rename = "<=")]
    LessThanOrEqual,
    #[serde(rename = ">")]
    GreaterThan,
    #[serde(rename = ">=")]
    GreaterThanOrEqual,
    #[serde(rename = "==")]
    Equal,
}

/// Right-hand-side type for alert conditions.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum AlertRhsType {
    #[serde(rename = "constant")]
    Constant,
    #[serde(rename = "instrument")]
    Instrument,
}

/// Core alert DTO returned by list/get/create/modify endpoints.
#[derive(Debug, Serialize, Deserialize)]
pub struct Alert {
    #[serde(rename = "type")]
    pub alert_type: AlertType,
    pub user_id: String,
    pub uuid: String,
    pub name: String,
    pub status: AlertStatus,
    pub disabled_reason: String,
    pub lhs_attribute: String,
    pub lhs_exchange: Exchange,
    pub lhs_tradingsymbol: String,
    pub operator: AlertOperator,
    pub rhs_type: AlertRhsType,
    pub rhs_attribute: String,
    #[serde(default)]
    pub rhs_exchange: Option<Exchange>,
    pub rhs_tradingsymbol: String,
    #[serde(default)]
    pub rhs_constant: Option<f64>,
    pub alert_count: i64,
    #[serde(default)]
    pub basket: Option<AlertBasket>,
    pub created_at: String,
    pub updated_at: String,
}

/// Basket configuration for ATO alerts.
#[derive(Debug, Serialize, Deserialize)]
pub struct AlertBasket {
    pub items: Vec<AlertBasketItem>,
}

/// Individual entry in `basket.items`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AlertBasketItem {
    #[serde(default)]
    pub id: Option<i64>,
    pub tradingsymbol: String,
    pub exchange: Exchange,
    #[serde(default)]
    pub instrument_token: Option<u64>,
    pub weight: i64,
    pub params: AlertBasketParams,
}

/// Order parameters inside an ATO alert basket item.
#[derive(Debug, Serialize, Deserialize)]
pub struct AlertBasketParams {
    pub validity: String,
    pub validity_ttl: i64,
    pub variety: String,
    pub product: ProductType,
    pub order_type: OrderType,
    pub transaction_type: TransactionType,
    pub quantity: i64,
    pub disclosed_quantity: i64,
    pub price: f64,
    pub trigger_price: f64,
    pub squareoff: f64,
    pub stoploss: f64,
    pub trailing_stoploss: f64,
    #[serde(default)]
    pub gtt: Option<AlertBasketGttMeta>,
    pub tags: Vec<String>,
}

/// Nested GTT metadata inside basket params.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlertBasketGttMeta {
    pub target: f64,
    pub stoploss: f64,
}

/// Entry from `/alerts/{uuid}/history`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlertHistoryEntry {
    pub uuid: String,
    #[serde(rename = "type")]
    pub alert_type: AlertType,
    pub meta: Vec<AlertHistoryMeta>,
    pub condition: String,
    pub created_at: String,
    #[serde(default)]
    pub order_meta: Option<serde_json::Value>,
}

/// Market snapshot metadata within a history entry.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlertHistoryMeta {
    pub instrument_token: u64,
    pub tradingsymbol: String,
    pub timestamp: String,
    pub last_price: f64,
    pub ohlc: AlertHistoryOhlc,
    pub net_change: f64,
    pub exchange: Exchange,
    pub last_trade_time: String,
    pub last_quantity: i64,
    pub buy_quantity: i64,
    pub sell_quantity: i64,
    pub volume: i64,
    pub volume_tick: i64,
    pub average_price: f64,
    pub oi: f64,
    pub oi_day_high: f64,
    pub oi_day_low: f64,
    pub lower_circuit_limit: f64,
    pub upper_circuit_limit: f64,
}

/// OHLC data for history entries.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlertHistoryOhlc {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// Request payload for creating or modifying an alert.
#[derive(Debug, Serialize, Deserialize)]
pub struct AlertRequest {
    pub name: String,
    #[serde(rename = "type")]
    pub alert_type: AlertType,
    pub lhs_exchange: Exchange,
    pub lhs_tradingsymbol: String,
    pub lhs_attribute: String,
    pub operator: AlertOperator,
    pub rhs_type: AlertRhsType,
    #[serde(default)]
    pub rhs_constant: Option<f64>,
    #[serde(default)]
    pub rhs_exchange: Option<Exchange>,
    #[serde(default)]
    pub rhs_tradingsymbol: Option<String>,
    #[serde(default)]
    pub rhs_attribute: Option<String>,
    #[serde(default)]
    pub basket: Option<AlertBasket>,
}

/// Optional query parameters for listing alerts.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AlertListFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AlertStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
}
