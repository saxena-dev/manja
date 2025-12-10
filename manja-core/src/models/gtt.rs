use serde::{Deserialize, Serialize};

use crate::models::{
    Exchange, OrderType, ProductType, TransactionType,
};

/// Type of GTT trigger.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum GttType {
    #[serde(rename = "single")]
    Single,
    #[serde(rename = "two-leg")]
    TwoLeg,
}

/// Status of a GTT trigger.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum GttStatus {
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "triggered")]
    Triggered,
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "expired")]
    Expired,
    #[serde(rename = "cancelled")]
    Cancelled,
    #[serde(rename = "rejected")]
    Rejected,
    #[serde(rename = "deleted")]
    Deleted,
}

/// Condition payload for a GTT trigger.
#[derive(Debug, Serialize, Deserialize)]
pub struct GttCondition {
    pub exchange: Exchange,
    pub tradingsymbol: String,
    pub trigger_values: Vec<f64>,
    pub last_price: f64,
    #[serde(default)]
    pub instrument_token: Option<u64>,
}

/// Order entry inside the GTT `orders` list.
#[derive(Debug, Serialize, Deserialize)]
pub struct GttOrderParams {
    pub exchange: Exchange,
    pub tradingsymbol: String,
    pub transaction_type: TransactionType,
    pub quantity: i64,
    pub order_type: OrderType,
    pub product: ProductType,
    pub price: f64,
    #[serde(default)]
    pub result: Option<GttOrderResult>,
}

/// Embedded order result information for a GTT order.
#[derive(Debug, Serialize, Deserialize)]
pub struct GttOrderResult {
    pub account_id: String,
    pub exchange: Exchange,
    pub tradingsymbol: String,
    pub validity: String,
    pub product: ProductType,
    pub order_type: OrderType,
    pub transaction_type: TransactionType,
    pub quantity: i64,
    pub price: f64,
    #[serde(default)]
    pub meta: Option<String>,
    pub timestamp: String,
    pub triggered_at: f64,
    #[serde(default)]
    pub order_result: Option<GttOrderExecutionResult>,
}

/// Execution result for a GTT order.
#[derive(Debug, Serialize, Deserialize)]
pub struct GttOrderExecutionResult {
    pub status: String,
    pub order_id: String,
    #[serde(default)]
    pub rejection_reason: Option<String>,
}

/// Full trigger object returned by list/get endpoints.
#[derive(Debug, Serialize, Deserialize)]
pub struct GttTrigger {
    pub id: i64,
    pub user_id: String,
    #[serde(default)]
    pub parent_trigger: Option<i64>,
    #[serde(rename = "type")]
    pub gtt_type: GttType,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: String,
    pub status: GttStatus,
    pub condition: GttCondition,
    pub orders: Vec<GttOrderParams>,
    #[serde(default)]
    pub meta: serde_json::Value,
}

/// Wrapper for create/modify/delete responses.
#[derive(Debug, Serialize, Deserialize)]
pub struct GttTriggerId {
    pub trigger_id: i64,
}

/// Request payload for creating or modifying a GTT trigger.
#[derive(Debug, Serialize, Deserialize)]
pub struct GttTriggerRequest {
    #[serde(rename = "type")]
    pub gtt_type: GttType,
    pub condition: GttCondition,
    pub orders: Vec<GttOrderParams>,
}
