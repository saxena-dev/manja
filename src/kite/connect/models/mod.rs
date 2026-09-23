//! Data types for interacting with Kite Connect (HTTP) API.
//!
//! This module defines the data models used in Kite Connect API. These models
//! represent the various structures used in API requests and responses, making
//! it easier to work with Kite Connect API in a type-safe manner.
//!
//! [`KiteApiResponse<T>`] is the response envelope; its `data` holds the
//! endpoint's type. For example, a successful `Session::exchange` yields
//! `KiteApiResponse<UserSession>`, where [`UserSession`] holds the
//! secret-wrapped tokens returned by the token-exchange endpoint.
//!
//! These models are compiled in every feature build; the resource APIs that
//! return them need the `http` feature.
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
#[cfg(feature = "http")]
pub(crate) use order::check_order_id;
pub use order::{
    ModifyOrderRequest, Order, OrderReceipt, PlaceOrderRequest, RequestError, SliceError,
    SliceResult, Trade,
};
#[allow(unused_imports)]
pub use order_enums::{
    OrderStatus, OrderType, OrderValidity, OrderVariety, ProductType, TransactionType,
};

/// Models for historical candle data: `/instruments/historical/`.
///
mod historical;
pub use historical::{Candle, CandleInterval, HistoricalData, HistoricalRequest};

/// Models for the `/gtt/` API group: Good Till Triggered orders.
///
mod gtt;
pub use gtt::{
    GttCondition, GttOrder, GttOrderOutcome, GttOrderRequest, GttOrderResult, GttReceipt,
    GttRequest, GttStatus, GttTrigger, GttType,
};

/// Models for the `/portfolio/` API group, managing holdings and positions.
///
mod portfolio;
pub use portfolio::{
    Auction, Holding, HoldingMtf, Position, PositionConversionRequest, PositionType, Positions,
};

/// Models for the `/instruments/` and `/quote/` API group, providing market data
/// and instrument information.
///
mod market;
#[allow(unused_imports)]
pub use market::{
    Depth, DepthLevel, FullQuote, Instrument, InstrumentType, KiteQuote, LTPQuote, OHLCQuote,
    QuoteMode, Quotes, OHLC,
};

/// Models for the `/margins/` and `/charges/` API group, dealing with margin
/// requirements and charges.
///
mod margins;
pub use margins::{
    BasketMargin, Charges, OrderCharges, OrderChargesRequest, OrderMargin, OrderMarginRequest, GST,
    PNL,
};

/// The partial order-update (postback) DTO, shared with the common protocol
/// slice and the text decoder. It is the crate's only order-update type.
pub use crate::kite::protocol::OrderUpdate;

/// Enumerations for exchanges supported by Kite Connect API.
mod exchange;
pub use exchange::Exchange;
