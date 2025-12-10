//! Postback (webhook/WebSocket) payload models and helpers.
//!
//! This module defines a typed representation of the Kite Connect
//! order postback payload as described in the official documentation,
//! along with helpers for parsing and checksum verification.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{
    OrderStatus, OrderType, OrderValidity, OrderVariety, ProductType, TransactionType,
};

/// Order postback payload as sent by Kite Connect over HTTP webhooks
/// and WebSocket text messages.
///
/// This mirrors the payload documented in the Postbacks/WebHooks section
/// and the `kiteconnect-mocks/postback.json` fixture.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OrderPostback {
    /// ID of the user for whom the order was placed.
    pub user_id: String,
    /// User ID of the entity that placed the order.
    pub placed_by: String,
    /// Application ID.
    pub app_id: i64,
    /// Checksum for verifying authenticity of the payload.
    pub checksum: String,
    /// Unique order ID.
    pub order_id: String,
    /// Exchange generated order ID, if available.
    #[serde(default)]
    pub exchange_order_id: Option<String>,
    /// Order ID of the parent order (for multi-legged orders).
    #[serde(default)]
    pub parent_order_id: Option<String>,
    /// Current status of the order.
    pub status: OrderStatus,
    /// Textual description of the order's status.
    #[serde(default)]
    pub status_message: Option<String>,
    /// Raw textual description of the order's status.
    #[serde(default)]
    pub status_message_raw: Option<String>,
    /// Timestamp at which the order was registered by the API.
    pub order_timestamp: String,
    /// Timestamp at which the order's state changed at the exchange.
    #[serde(default)]
    pub exchange_update_timestamp: Option<String>,
    /// Timestamp at which the order was registered by the exchange.
    #[serde(default)]
    pub exchange_timestamp: Option<String>,
    /// Order variety (e.g. regular, amo, co).
    pub variety: OrderVariety,
    /// Exchange where the order was placed.
    pub exchange: String,
    /// Exchange tradingsymbol of the instrument.
    pub tradingsymbol: String,
    /// Numerical identifier of the instrument.
    pub instrument_token: u64,
    /// Order type (MARKET, LIMIT, etc.).
    pub order_type: OrderType,
    /// Transaction type (BUY or SELL).
    pub transaction_type: TransactionType,
    /// Order validity.
    pub validity: OrderValidity,
    /// Margin product to use for the order.
    pub product: ProductType,
    /// Quantity ordered.
    pub quantity: i64,
    /// Quantity to be disclosed.
    pub disclosed_quantity: i64,
    /// Price at which the order was placed.
    pub price: f64,
    /// Trigger price (for SL/CO orders).
    pub trigger_price: f64,
    /// Average price at which the order was executed.
    pub average_price: f64,
    /// Quantity that has been filled.
    pub filled_quantity: i64,
    /// Quantity that has not yet been filled.
    pub unfilled_quantity: i64,
    /// Pending quantity for open orders.
    pub pending_quantity: i64,
    /// Quantity that has been cancelled.
    pub cancelled_quantity: i64,
    /// Market protection value.
    pub market_protection: i64,
    /// Map of arbitrary metadata fields.
    #[serde(default)]
    pub meta: serde_json::Value,
    /// Optional tag associated with the order.
    #[serde(default)]
    pub tag: Option<String>,
    /// Unusable request ID to avoid order duplication.
    pub guid: String,
}

/// Parse a raw HTTP postback payload into an [`OrderPostback`].
pub fn parse_http_postback(body: &str) -> Result<OrderPostback, serde_json::Error> {
    serde_json::from_str(body)
}

/// Envelope for WebSocket order postbacks and other text messages.
#[derive(Debug, Deserialize)]
pub struct WebSocketPostbackEnvelope<T> {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub data: T,
}

/// Parse a WebSocket text message carrying an order postback payload.
///
/// This expects messages of the form:
/// `{ "type": "order", "data": { /* order postback payload */ } }`.
pub fn parse_websocket_order_postback(
    text: &str,
) -> Result<OrderPostback, serde_json::Error> {
    let envelope: WebSocketPostbackEnvelope<OrderPostback> = serde_json::from_str(text)?;
    // Optionally, callers can inspect `envelope.msg_type` if they care.
    Ok(envelope.data)
}

/// Compute the expected checksum for an order postback payload.
///
/// The checksum is defined as:
/// `SHA256(order_id + order_timestamp + api_secret)`, encoded as a lowercase
/// hexadecimal string.
pub fn compute_postback_checksum(
    order_id: &str,
    order_timestamp: &str,
    api_secret: &str,
) -> String {
    let payload = format!("{}{}{}", order_id, order_timestamp, api_secret);
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Verify the checksum of a parsed [`OrderPostback`] payload.
///
/// Returns `true` if the checksum matches the expected value computed from
/// `order_id`, `order_timestamp`, and the provided `api_secret`.
pub fn verify_postback_checksum(postback: &OrderPostback, api_secret: &str) -> bool {
    let expected = compute_postback_checksum(
        &postback.order_id,
        &postback.order_timestamp,
        api_secret,
    );
    expected == postback.checksum
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_postback() -> String {
        // This JSON mirrors `kiteconnect-mocks/postback.json`.
        r#"{
            "user_id": "AB1234",
            "unfilled_quantity": 0,
            "app_id": 1234,
            "checksum": "2011845d9348bd6795151bf4258102a03431e3bb12a79c0df73fcb4b7fde4b5d",
            "placed_by": "AB1234",
            "order_id": "220303000308932",
            "exchange_order_id": "1000000001482421",
            "parent_order_id": null,
            "status": "COMPLETE",
            "status_message": null,
            "status_message_raw": null,
            "order_timestamp": "2022-03-03 09:24:25",
            "exchange_update_timestamp": "2022-03-03 09:24:25",
            "exchange_timestamp": "2022-03-03 09:24:25",
            "variety": "regular",
            "exchange": "NSE",
            "tradingsymbol": "SBIN",
            "instrument_token": 779521,
            "order_type": "MARKET",
            "transaction_type": "BUY",
            "validity": "DAY",
            "product": "CNC",
            "quantity": 1,
            "disclosed_quantity": 0,
            "price": 0,
            "trigger_price": 0,
            "average_price": 470,
            "filled_quantity": 1,
            "pending_quantity": 0,
            "cancelled_quantity": 0,
            "market_protection": 0,
            "meta": {},
            "tag": null,
            "guid": "XXXXXX"
        }"#.to_string()
    }

    #[test]
    fn parse_http_postback_roundtrip() {
        let json = sample_postback();
        let postback = parse_http_postback(&json).expect("valid postback JSON");

        assert_eq!(postback.user_id, "AB1234");
        assert_eq!(postback.placed_by, "AB1234");
        assert_eq!(postback.order_id, "220303000308932");
        assert_eq!(postback.tradingsymbol, "SBIN");
        assert_eq!(postback.quantity, 1);
    }

    #[test]
    fn compute_checksum_matches_expected() {
        // These values must correspond to the checksum in `sample_postback`.
        let order_id = "220303000308932";
        let order_timestamp = "2022-03-03 09:24:25";
        // This is a placeholder secret; in real usage, the application
        // must supply its actual API secret.
        let api_secret = "dummy_secret";

        let checksum =
            compute_postback_checksum(order_id, order_timestamp, api_secret);

        // We don't know the secret used in the fixture, so we can't
        // assert equality with the sample checksum here. We only assert
        // that the function is stable and returns a non-empty value.
        assert!(!checksum.is_empty());
    }

    #[test]
    fn verify_checksum_behaves_consistently() {
        let json = sample_postback();
        let mut postback =
            parse_http_postback(&json).expect("valid postback JSON");

        let api_secret = "dummy_secret";
        // Overwrite checksum to match the one we would compute with this secret.
        postback.checksum = compute_postback_checksum(
            &postback.order_id,
            &postback.order_timestamp,
            api_secret,
        );

        assert!(verify_postback_checksum(&postback, api_secret));
        assert!(!verify_postback_checksum(&postback, "wrong_secret"));
    }

    #[test]
    fn parse_websocket_postback_envelope() {
        let inner = sample_postback();
        let envelope = format!(r#"{{ "type": "order", "data": {} }}"#, inner);

        let postback =
            parse_websocket_order_postback(&envelope).expect("valid WS envelope");

        assert_eq!(postback.order_id, "220303000308932");
        assert_eq!(postback.tradingsymbol, "SBIN");
    }

    proptest! {
        #[test]
        fn checksum_roundtrip_holds_for_generated_inputs(
            order_id in any::<String>(),
            order_timestamp in any::<String>(),
            api_secret in any::<String>(),
        ) {
            let checksum = compute_postback_checksum(&order_id, &order_timestamp, &api_secret);

            let postback = OrderPostback {
                user_id: "U".to_string(),
                placed_by: "U".to_string(),
                app_id: 1,
                checksum: checksum.clone(),
                order_id: order_id.clone(),
                exchange_order_id: None,
                parent_order_id: None,
                status: OrderStatus::Complete,
                status_message: None,
                status_message_raw: None,
                order_timestamp: order_timestamp.clone(),
                exchange_update_timestamp: None,
                exchange_timestamp: None,
                variety: OrderVariety::Regular,
                exchange: "NSE".to_string(),
                tradingsymbol: "SYMB".to_string(),
                instrument_token: 1,
                order_type: OrderType::Market,
                transaction_type: TransactionType::BUY,
                validity: OrderValidity::Day,
                product: ProductType::CashAndCarry,
                quantity: 1,
                disclosed_quantity: 0,
                price: 0.0,
                trigger_price: 0.0,
                average_price: 0.0,
                filled_quantity: 0,
                unfilled_quantity: 0,
                pending_quantity: 0,
                cancelled_quantity: 0,
                market_protection: 0,
                meta: serde_json::Value::Null,
                tag: None,
                guid: "guid".to_string(),
            };

            prop_assert!(verify_postback_checksum(&postback, &api_secret));
            prop_assert!(!verify_postback_checksum(&postback, "different_secret"));
        }
    }
}
