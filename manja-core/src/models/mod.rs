//! Data types for interacting with Kite Connect (HTTP) API.
//!
//! This module defines the data models used in Kite Connect API. These models
//! represent the various structures used in API requests and responses, making
//! it easier to work with Kite Connect API in a type-safe manner.
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
pub use portfolio::{Auction, Holding, Position, PositionConversionRequest};

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
