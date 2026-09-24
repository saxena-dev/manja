//! Everything manja offers, grouped by what it is for.
//!
//! - [`connect`]: the HTTP API. [`HTTPClient`](connect::client::HTTPClient)
//!   and its resources need the `http` feature; credentials and response
//!   models are always available.
//! - [`ticker`]: the WebSocket ticker for live market data and order
//!   updates. Needs the `ticker` feature.
//! - [`decoder`]: decoding of the ticker's binary and text messages, live or
//!   captured. Needs the `decoder` feature.
//! - [`error`]: the errors every call can return.
//! - [`obs`]: opt-in metrics, tracing spans and diagnostics.
//! - [`protocol`]: checked types for the values Kite sends and accepts, such
//!   as instrument tokens, order IDs, quantities and scaled prices.
//! - [`envelope`]: the provenance record wrapped around every ticker
//!   message.
//!
pub mod connect;
#[cfg(feature = "decoder")]
pub mod decoder;
pub mod envelope;
pub mod error;
pub mod obs;
pub mod protocol;
#[cfg(feature = "ticker")]
pub mod ticker;
