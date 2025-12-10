//! Types for interacting with Kite Connect WebSocket API.
//!
//! This module defines the data models used for interacting with Kite Connect
//! streaming API. It includes the `Mode` enum for specifying the streaming data
//! mode and the `TickerRequest` struct for creating WebSocket requests.

mod mode;
pub use mode::Mode;

mod request;
pub use request::TickerRequest;

