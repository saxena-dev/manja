//! The Kite Connect WebSocket ticker.
//!
//! This module is compiled with the `ticker` feature.
//!
//! # Raw and typed streams
//!
//! The ticker, built in `actor` (`docs/contract.md` §2.5), is one
//! socket-owning task with typed commands, a status handle and a supervised task
//! guard. Its primary receiver yields raw observations and lifecycle events
//! before any interpretation, and it needs no decoder. Typed market events come
//! from composing it with the `decoder` feature, which decodes those raw
//! observations; typed events never replace the raw stream.
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
