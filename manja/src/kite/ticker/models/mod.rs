//! Types for interacting with Kite Connect WebSocket API.
//!
//! This module defines the data models used for interacting with Kite Connect
//! streaming API. It includes the `Mode` enum for specifying the streaming data
//! mode and the `TickerRequest` struct for creating WebSocket requests to subscribe,
//! unsubscribe, and set modes for instrument tokens.
//!
//! # Submodules
//!
//! - `mode`: Defines the `Mode` enum, which represents the different modes in which
//!     data packets can be streamed.
//! - `request`: Defines the `TickerRequest` struct, which represents the structure
//!     of a WebSocket request.
//!
mod mode;
pub use mode::Mode;

mod request;
pub use request::TickerRequest;
