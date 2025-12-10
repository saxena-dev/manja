//! Data types for interacting with Kite Connect (HTTP) API.
//!
//! This module defines the data models used in Kite Connect API. These models
//! represent the various structures used in API requests and responses, making
//! it easier to work with Kite Connect API in a type-safe manner.
//!
//! At the workspace level, the `manja` facade re-exports a **smaller, curated
//! subset** of these types at its crate root (`manja::*`) – primarily
//! [`KiteApiResponse`], user/session models, core order types, and a handful of
//! commonly used market, margin, and mutual fund types.
//!
//! The full set of HTTP models, including advanced/rarely used types (for
//! example, alert/GTT payloads and postback helpers), remains available here
//! under [`manja_core::models`] and is also re-exported under
//! [`manja::kite::connect::models`]. When in doubt:
//!
//! - Use types from `manja`'s crate root for common workflows.
//! - Import additional or specialised types directly from `manja_core::models`
//!   or `manja::kite::connect::models`.
//!
//! [`KiteApiResponse<T>`] is the wrapper struct that represents a response from
//! Kite Connect API and is a good starting point to dig deeper. The generic type
//! `T` is the specific data structure returned from an API endpoint. For example,
//! the type `T` in the code below is [`UserSession`] representing the information
//! returned by the API from the endpoint pointed at by the method `generate_session()`.
//!
//! ```ignore
//! // Login flow I: request token
//! let request_token: String = format!("xxx");
//!
//! // Login flow II: user session
//! let _kite_session: KiteApiResponse<UserSession> = manja_client
//!    .session()
//!    .generate_session(&request_token)
//!    .await?;
//! ```
//!
//! A minimal example that does not depend on external services:
//!
//! ```rust
//! use manja_core::models::KiteApiResponse;
//!
//! let response: KiteApiResponse<u32> = KiteApiResponse {
//!     status: "success".to_string(),
//!     data: Some(42),
//!     message: None,
//!     error_type: None,
//! };
//!
//! assert_eq!(response.data, Some(42));
//! ```
//!
use serde::{Deserialize, Serialize};

/// Represents the default response structure used by Kite Connect API.
///
/// The generic type `T` is typically a `HashMap` but can be any type that the
/// specific API response requires.
///
#[derive(Serialize, Deserialize, Debug)]
pub struct KiteApiResponse<T> {
    /// The status of the API response (e.g., "success" or "error").
    pub status: String,
    /// The actual data returned by the API, if any.
    pub data: Option<T>,
    /// An optional message providing additional information about the response.
    pub message: Option<String>,
    /// An optional error type string, present if the response indicates an error.
    pub error_type: Option<String>,
}

/// Models for the `/session/` API group, including user session management.
///
mod session;
pub use session::UserSession;

/// Models for the `/user/` API group, handling user-specific data and settings.
///
mod user;
#[allow(unused_imports)]
pub use user::{Available, Segment, SegmentKind, UserMargins, UserProfile, Utilised};

/// Models for the `/orders/` API group, facilitating order placement, modification,
/// and status checks.
///
mod order;
mod order_enums;
pub use order::{Order, OrderReceipt, Trade};
#[allow(unused_imports)]
pub use order_enums::{
    OrderStatus, OrderType, OrderValidity, OrderVariety, ProductType, TransactionType,
};

/// Models for the `/portfolio/` API group, managing holdings and positions.
///
mod portfolio;
pub use portfolio::{
    Auction, Holding, HoldingAuthorisationItem, HoldingsAuthorisationResponse, Position,
    PositionConversionRequest, Positions,
};

/// Models for the `/instruments/` and `/quote/` API group, providing market
/// data and instrument information.
///
mod market;
pub use market::KiteQuote;
#[allow(unused_imports)]
pub use market::{FullQuote, Instrument, LTPQuote, OHLCQuote, QuoteMode};

/// Models for the `/margins/` and `/charges/` API group, dealing with margin
/// requirements and charges.
///
mod margins;
#[allow(unused_imports)]
pub use margins::{
    BasketMargin, Charges, OrderCharges, OrderChargesRequest, OrderMargin, OrderMarginRequest, GST,
    PNL,
};

/// Enumerations for exchanges supported by Kite Connect API.
mod exchange;
pub use exchange::Exchange;

/// Models for GTT (Good Till Triggered) orders.
mod gtt;
#[allow(unused_imports)]
pub use gtt::{
    GttCondition, GttOrderExecutionResult, GttOrderParams, GttOrderResult, GttStatus, GttTrigger,
    GttTriggerId, GttTriggerRequest, GttType,
};

/// Models for price and ATO alerts.
mod alerts;
#[allow(unused_imports)]
pub use alerts::{
    Alert, AlertBasket, AlertBasketGttMeta, AlertBasketItem, AlertBasketParams, AlertHistoryEntry,
    AlertHistoryMeta, AlertHistoryOhlc, AlertListFilter, AlertOperator, AlertRequest,
    AlertRhsType, AlertStatus, AlertType,
};

/// Models for historical OHLCV(+OI) data.
mod historical;
#[allow(unused_imports)]
pub use historical::{HistoricalCandle, HistoricalData, HistoricalInterval};

/// Models and helpers for order postbacks (webhooks/WebSocket).
mod postbacks;
#[allow(unused_imports)]
pub use postbacks::{
    compute_postback_checksum, parse_http_postback, parse_websocket_order_postback,
    verify_postback_checksum, OrderPostback, WebSocketPostbackEnvelope,
};

/// Models for mutual funds (Coin) APIs.
mod mutual_funds;
#[allow(unused_imports)]
pub use mutual_funds::{
    MfHolding, MfInstrument, MfOrder, MfOrderId, MfOrderRequest, MfSip, MfSipCreateRequest,
    MfSipId, MfSipModifyRequest,
};

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestPayload {
        value: i64,
        note: Option<String>,
    }

    proptest! {
        #[test]
        fn kite_api_response_serde_roundtrip(
            status in any::<String>(),
            data in proptest::option::of(
                (any::<i64>(), proptest::option::of(any::<String>()))
                    .prop_map(|(value, note)| TestPayload { value, note })
            ),
            message in proptest::option::of(any::<String>()),
            error_type in proptest::option::of(any::<String>()),
        ) {
            let response = KiteApiResponse {
                status: status.clone(),
                data: data.clone(),
                message: message.clone(),
                error_type: error_type.clone(),
            };

            let json = serde_json::to_string(&response).expect("serialize KiteApiResponse");
            let decoded: KiteApiResponse<TestPayload> =
                serde_json::from_str(&json).expect("deserialize KiteApiResponse");

            prop_assert_eq!(decoded.status, status);
            prop_assert_eq!(decoded.data, data);
            prop_assert_eq!(decoded.message, message);
            prop_assert_eq!(decoded.error_type, error_type);
        }
    }
}
