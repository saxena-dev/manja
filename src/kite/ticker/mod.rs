//! Async WebSocket client and additional functionality.
//!
//! This module is compiled with the `ticker` feature. It holds the legacy
//! WebSocket client and its replacement, `actor`.
//!
//! # Legacy client
//!
//! [`WebSocketClient`] connects to the Kite Connect WebSocket API with the
//! credentials and instrument tokens held in a [`StreamState`] and yields the
//! `tungstenite` messages it receives, uninterpreted. It makes no readiness,
//! reconnection or subscription-restoration guarantee: polling reads the
//! current socket directly, and the initial requests it sends are built by
//! [`TickerRequest::subscribe_with_mode`], which creates a `mode` action, not a
//! `subscribe` action.
//!
//! Its migration disposition, set in the SDK contract, is deprecation with
//! corrected documentation, not in-place repair, and the same disposition
//! covers `subscribe_with_mode`. The legacy client stays compiled and exported
//! during migration, its stream item type does not change, and it is removed
//! only in a later documented breaking release after its replacement ships.
//!
//! # Raw and typed streams
//!
//! The SDK contract specifies the replacement, built in `actor`, as one
//! socket-owning task with typed commands, a status handle and a supervised task
//! guard. Its primary receiver yields raw observations and lifecycle events
//! before any interpretation, and it needs no decoder. Typed market events come
//! from composing it with the `decoder` feature, which decodes those raw
//! observations; typed events never replace the raw stream.
//!

// The single-owner ticker: owner, subscriptions, lifecycle, delivery and
// status.
pub mod actor;

// Contains the `WebSocketClient` and `TickerStream` structs, which are used to
// connect to the WebSocket API and handle data streaming.
mod client;
#[allow(unused_imports)]
pub use client::{TickerStream, WebSocketClient};

// Defines the structures for managing the stream state and credentials, including
// `KiteStreamCredentials` and `StreamState`.
mod stream;
#[allow(unused_imports)]
pub use stream::{KiteStreamCredentials, StreamState};

// Contains data models and request types like `Mode` and `TickerRequest` used
// for interacting with the WebSocket API.
mod models;
#[allow(unused_imports)]
pub use models::{Mode, TickerRequest};
