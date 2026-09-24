//! The partial order-update DTO.
//!
//! [`OrderUpdate`] is the payload of an order postback
//! (`kite:postbacks.md:13-93`) and of the `data`
//! field of a WebSocket `order` text message (`kite:websocket.md:167-182`). It is
//! the crate's single order-update type, shared by HTTP-side postback
//! handling and the text decoder.
//!
//! Only `order_id` is required. Every other field is optional, because an
//! `UPDATE` (a modification or partial fill of an open order) need not carry
//! every order-list field. Status, type and product strings are
//! [`Inbound`] values: an unknown value is preserved with its original text,
//! never coerced into a recognized status, and never convertible into an
//! outbound command.
//!
//! An update is broker evidence about one order at one moment. It is not a
//! fill confirmation unless its own fields say so, and combining updates into
//! account state is the application's job. Checksum verification of HTTP
//! postbacks (`kite:postbacks.md:54-56`) needs the API secret and belongs to a
//! webhook-ingestion boundary this crate does not provide.
//!
use std::fmt;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::kite::connect::models::{
    Exchange, OrderStatus, OrderType, OrderValidity, OrderVariety, ProductType, TransactionType,
};
use crate::kite::protocol::datetime::serde_opt_datetime;
use crate::kite::protocol::{Inbound, InstrumentToken, OrderId};

/// The postback `checksum` field. It is derived from the API secret, so
/// `Debug` redacts it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PostbackChecksum(String);

impl PostbackChecksum {
    /// The checksum text, for an application's own verification.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PostbackChecksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostbackChecksum(<redacted>)")
    }
}

/// A partial order update: an order postback payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OrderUpdate {
    /// Unique order ID. The only required field.
    pub order_id: OrderId,
    /// Exchange order ID; `null` for orders that never reached the exchange.
    #[serde(default)]
    pub exchange_order_id: Option<String>,
    /// Parent order ID, for multi-legged orders.
    #[serde(default)]
    pub parent_order_id: Option<OrderId>,
    /// Current status. Documented postback values are COMPLETE, REJECTED,
    /// CANCELLED and UPDATE.
    #[serde(default)]
    pub status: Option<Inbound<OrderStatus>>,
    /// Textual description of the status.
    #[serde(default)]
    pub status_message: Option<String>,
    /// Raw status description from the OMS.
    #[serde(default)]
    pub status_message_raw: Option<String>,
    /// User for whom the order was placed.
    #[serde(default)]
    pub user_id: Option<String>,
    /// User who placed the order.
    #[serde(default)]
    pub placed_by: Option<String>,
    /// Kite Connect app ID.
    #[serde(default)]
    pub app_id: Option<i64>,
    /// Exchange tradingsymbol.
    #[serde(default)]
    pub tradingsymbol: Option<String>,
    /// Instrument token.
    #[serde(default)]
    pub instrument_token: Option<InstrumentToken>,
    /// Exchange.
    #[serde(default)]
    pub exchange: Option<Inbound<Exchange>>,
    /// Order variety.
    #[serde(default)]
    pub variety: Option<Inbound<OrderVariety>>,
    /// Order type.
    #[serde(default)]
    pub order_type: Option<Inbound<OrderType>>,
    /// BUY or SELL.
    #[serde(default)]
    pub transaction_type: Option<Inbound<TransactionType>>,
    /// Order validity.
    #[serde(default)]
    pub validity: Option<Inbound<OrderValidity>>,
    /// Margin product.
    #[serde(default)]
    pub product: Option<Inbound<ProductType>>,
    /// Quantity ordered.
    #[serde(default)]
    pub quantity: Option<i64>,
    /// Quantity disclosed to the exchange order book.
    #[serde(default)]
    pub disclosed_quantity: Option<i64>,
    /// Quantity filled so far.
    #[serde(default)]
    pub filled_quantity: Option<i64>,
    /// Quantity not filled.
    #[serde(default)]
    pub unfilled_quantity: Option<i64>,
    /// Quantity pending for an open order.
    #[serde(default)]
    pub pending_quantity: Option<i64>,
    /// Quantity cancelled.
    #[serde(default)]
    pub cancelled_quantity: Option<i64>,
    /// Order price (LIMIT orders), as the broker's JSON number.
    #[serde(default)]
    pub price: Option<f64>,
    /// Trigger price (SL, SL-M, CO orders).
    #[serde(default)]
    pub trigger_price: Option<f64>,
    /// Average executed price.
    #[serde(default)]
    pub average_price: Option<f64>,
    /// Market protection setting.
    #[serde(default)]
    pub market_protection: Option<f64>,
    /// When the API registered the order (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub order_timestamp: Option<DateTime<FixedOffset>>,
    /// When the exchange registered the order (IST); `null` if it never did.
    #[serde(default, with = "serde_opt_datetime")]
    pub exchange_timestamp: Option<DateTime<FixedOffset>>,
    /// When the order's state changed at the exchange (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub exchange_update_timestamp: Option<DateTime<FixedOffset>>,
    /// Arbitrary fields the system may attach.
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
    /// Optional order tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Request GUID.
    #[serde(default)]
    pub guid: Option<String>,
    /// Postback checksum, `SHA-256(order_id + order_timestamp + api_secret)`.
    #[serde(default)]
    pub checksum: Option<PostbackChecksum>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_update_needs_only_the_order_id() {
        let u: OrderUpdate =
            serde_json::from_str(r#"{"order_id": "1", "status": "UPDATE"}"#).unwrap();
        assert_eq!(u.order_id, "1");
        assert_eq!(u.status.unwrap().known(), Some(&OrderStatus::Update));
        assert!(u.exchange_timestamp.is_none());
        assert!(serde_json::from_str::<OrderUpdate>(r#"{"status": "UPDATE"}"#).is_err());
    }

    #[test]
    fn unknown_statuses_are_preserved() {
        let u: OrderUpdate =
            serde_json::from_str(r#"{"order_id": "1", "status": "SOMETHING NEW"}"#).unwrap();
        let status = u.status.unwrap();
        assert_eq!(status.as_wire(), "SOMETHING NEW");
        assert!(OrderStatus::try_from(status).is_err());
    }

    #[test]
    fn malformed_timestamps_fail_visibly() {
        let bad = r#"{"order_id": "1", "order_timestamp": "03/03/2022"}"#;
        assert!(serde_json::from_str::<OrderUpdate>(bad).is_err());
    }

    #[test]
    fn checksum_is_redacted_in_debug() {
        let u: OrderUpdate =
            serde_json::from_str(r#"{"order_id": "1", "checksum": "deadbeef"}"#).unwrap();
        assert!(!format!("{u:?}").contains("deadbeef"));
        assert_eq!(u.checksum.unwrap().expose(), "deadbeef");
    }
}
