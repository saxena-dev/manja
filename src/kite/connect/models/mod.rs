//! Data types for interacting with Kite Connect (HTTP) API.
//!
//! This module defines the data models used in Kite Connect API. These models
//! represent the various structures used in API requests and responses, making
//! it easier to work with Kite Connect API in a type-safe manner.
//!
//! [`Response<T>`] is what every JSON endpoint returns: the endpoint's
//! `data`, which is always present, and the [`ResponseMeta`] of the call.
//! For example, a successful `Session::exchange` yields
//! `Response<UserSession>`, where [`UserSession`] holds the secret-wrapped
//! tokens returned by the token-exchange endpoint.
//!
//! These models are compiled in every feature build; the resource APIs that
//! return them need the `http` feature.
//!
use serde::{Deserialize, Serialize};

use crate::kite::envelope::{MonotonicElapsed, ReceiveTime, SourceIdentity};

/// A successful response: the endpoint's data and the facts of the call that
/// produced it.
///
/// A call returns `Ok(Response)` only when Kite answered with a 2xx status, a
/// `status: "success"` envelope (`kite:response-structure.md:15-28`) and data
/// of the expected shape, so `data` is the value itself, never an `Option`.
/// An error envelope or any other failure is a
/// [`ManjaError`](crate::kite::error::ManjaError) with Kite's error type and
/// message on its `HttpError`.
///
/// `KiteApiResponse`, the 0.2 envelope with its `status`, `Option<T>` data
/// and `error_type`, no longer exists:
///
/// ```compile_fail
/// let _ = std::mem::size_of::<manja::kite::connect::models::KiteApiResponse<()>>();
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response<T> {
    /// The endpoint's payload.
    pub data: T,
    /// The facts of the call: Kite's optional message, the HTTP status, the
    /// operation ID and the attempt number.
    pub meta: ResponseMeta,
}

impl<T> Response<T> {
    /// The same response with `f` applied to its data.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Response<U> {
        Response {
            data: f(self.data),
            meta: self.meta,
        }
    }
}

/// The facts of the call behind a [`Response`].
///
/// A later release may add fields, so this type is `#[non_exhaustive]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResponseMeta {
    /// Kite's optional informational message from the envelope.
    pub message: Option<String>,
    /// The HTTP status of the attempt that produced the response.
    pub http_status: u16,
    /// The operation's ID: the `operation_id` the `manja.http.operation`
    /// span records, unique within the client's observability scope.
    pub operation_id: u64,
    /// The number of the attempt that produced the response: 1 for a first
    /// try, 2 after one retry.
    pub attempt: u32,
    /// The client's source identity: producer, run and feed, the same type
    /// and frame the ticker's envelope carries, so HTTP evidence and ticker
    /// evidence from one process cite the same run.
    pub source: SourceIdentity,
    /// When the attempt that produced this response was dispatched: the
    /// last instant before its bytes left the process.
    pub sent_at: IngressStamp,
    /// When the response head (status and headers) arrived.
    pub received_at: IngressStamp,
}

/// One ingress or egress instant, in both of the envelope's clocks: wall
/// time for display and broker comparison, monotonic time for ordering
/// within a run.
///
/// The monotonic part counts from the client's run origin, so stamps from
/// one client and its clones are comparable with each other and with the
/// client's diagnostics, and not with another client's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IngressStamp {
    /// Wall-clock time, signed UTC Unix nanoseconds.
    pub wall: ReceiveTime,
    /// Monotonic time since the client's run origin.
    pub monotonic: MonotonicElapsed,
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
    SliceResult, Tag, Trade,
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
