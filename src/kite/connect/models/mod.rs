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

/// Kite's response envelope, as returned by every JSON endpoint
/// (`kite:response-structure.md`).
///
/// A call returns `Ok` only for a successful envelope, so on a value you
/// receive `status` is `"success"`, `data` is always `Some`, and `error_type`
/// is `None`. An error envelope is returned as a
/// [`ManjaError`](crate::kite::error::ManjaError) instead, with Kite's error
/// type and message available through its `HttpError`.
#[derive(Serialize, Deserialize, Debug)]
pub struct KiteApiResponse<T> {
    /// `"success"` on every response a call returns.
    pub status: String,
    /// The endpoint's payload. Always `Some` on a response a call returns.
    pub data: Option<T>,
    /// Kite's optional informational message.
    pub message: Option<String>,
    /// Always `None` on a response a call returns: errors are returned as
    /// errors.
    pub error_type: Option<String>,
}

/// Models for the `/session/` API group, including user session management.
///
mod session;
pub use session::UserSession;

/// Models for the `/user/` API group, handling user-specific data and settings.
///
mod user;
pub use user::{Available, Segment, SegmentKind, UserMargins, UserProfile, Utilised};

/// Models for the `/orders/` API group, facilitating order placement, modification,
/// and status checks.
///
mod order;
mod order_enums;
pub use order::{
    ModifyOrderRequest, Order, OrderReceipt, PlaceOrderRequest, RequestError, SliceError,
    SliceResult, Trade,
};
pub use order_enums::{
    OrderStatus, OrderType, OrderValidity, OrderVariety, ProductType, TransactionType,
};

/// Models for the `/mf/` API group: mutual fund orders, SIPs, holdings and
/// instruments.
///
mod mutual_funds;
pub use mutual_funds::{
    DividendType, MfHolding, MfInstrument, MfOrder, MfOrderStatus, MfOrderVariety, MfPlan,
    MfPurchaseType, MfSip, SchemeType, SipFrequency, SipStatus,
};

/// Models for historical candle data: `/instruments/historical/`.
///
mod historical;
pub use historical::{Candle, CandleInterval, HistoricalData, HistoricalRequest};

/// Row-by-row results of list responses, for the tolerant
/// `*_with_rejections` methods.
///
mod rows;
pub use rows::{Row, RowError, Rows};

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
    Auction, Holding, HoldingMtf, HoldingsAuthorisation, HoldingsAuthorisationRequest, Position,
    PositionConversionRequest, PositionType, Positions,
};

/// Models for the `/instruments/` and `/quote/` API group, providing market data
/// and instrument information.
///
mod market;
pub use market::{
    Depth, DepthLevel, FullQuote, Instrument, InstrumentType, KiteQuote, LTPQuote, OHLC, OHLCQuote,
    QuoteMode, Quotes,
};

/// Models for the `/margins/` and `/charges/` API group, dealing with margin
/// requirements and charges.
///
mod margins;
pub use margins::{
    BasketMargin, Charges, GST, OrderCharges, OrderChargesRequest, OrderMargin, OrderMarginRequest,
    PNL,
};

/// The partial order-update (postback) DTO, shared with the common protocol
/// slice and the text decoder. It is the crate's only order-update type.
pub use crate::kite::protocol::OrderUpdate;

/// Enumerations for exchanges supported by Kite Connect API.
mod exchange;
pub use exchange::Exchange;
