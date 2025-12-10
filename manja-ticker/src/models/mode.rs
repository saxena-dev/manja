//! Streaming mode enums for WebSocket API.
//!
//! This module defines the `Mode` enum, which represents the different modes
//! in which data packets can be streamed from Kite Connect WebSocket API.

use serde::{Deserialize, Serialize};

/// Represents the different modes in which packets can be streamed.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Mode {
    /// Full mode streams complete market depth and other data.
    #[serde(rename = "full")]
    Full,
    /// Quote mode streams top-of-book quotes.
    #[serde(rename = "quote")]
    Quote,
    /// LTP (Last Traded Price) mode streams only the last traded price.
    #[serde(rename = "ltp")]
    Ltp,
}

