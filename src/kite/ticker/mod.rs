//! The Kite Connect WebSocket ticker, for live market data and order
//! updates. Needs the `ticker` feature.
//!
//! Start with [`TickerBuilder`](actor::owner::TickerBuilder). It spawns one
//! background task that owns the connection, and gives you a handle for
//! commands and status and a stream of everything received, in order.
//!
//! The stream carries raw messages, exactly as Kite sent them, so nothing is
//! lost to a decoding error. With the `decoder` feature,
//! [`typed::TypedEvents`] delivers each message together with its decoding.
//! [`Mode`] chooses how much data Kite sends per instrument, and
//! [`TickerRequest`] builds the wire messages the ticker sends.
//!

// The single-owner ticker: owner, subscriptions, lifecycle, delivery and
// status.
pub mod actor;

// Typed events: the actor ticker composed with the decoder.
#[cfg(feature = "decoder")]
pub mod typed;

// Contains data models and request types like `Mode` and `TickerRequest` used
// for interacting with the WebSocket API.
mod models;
pub use models::{Mode, TickerRequest};
