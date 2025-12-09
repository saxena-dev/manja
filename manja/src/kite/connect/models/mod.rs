//! Data types for interacting with the Kite Connect (HTTP) API.
//!
//! This module is a thin facade over the shared models defined in the
//! `manja-core` crate. It re-exports the core types so that existing paths like
//! `crate::kite::connect::models::UserProfile` continue to work, while the
//! single source of truth for the definitions lives in `manja-core`.

pub use manja_core::models::{
    Auction, Available, BasketMargin, Charges, Exchange, FullQuote, GST, GttCondition,
    GttOrderExecutionResult, GttOrderParams, GttOrderResult, GttStatus, GttTrigger, GttTriggerId,
    GttTriggerRequest, GttType, Holding, Instrument, KiteApiResponse, LTPQuote, OHLCQuote, Order,
    OrderCharges, OrderChargesRequest, OrderMargin, OrderMarginRequest, OrderReceipt, OrderStatus,
    OrderType, OrderValidity, OrderVariety, PNL, Position, PositionConversionRequest, ProductType,
    QuoteMode, Segment, SegmentKind, Trade, TransactionType, UserMargins, UserProfile, UserSession,
    Utilised,
};

// Internal helper trait used by HTTP client and API groups.
pub(crate) use manja_core::models::KiteQuote;
