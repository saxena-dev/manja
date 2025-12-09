//! WebSocket request types for the ticker.
//!
//! This module provides the `TickerRequest` type used to interact with
//! Kite Connect streaming API. It includes enums for request actions and data,
//! and provides methods to subscribe, unsubscribe, and set modes for tokens.

use crate::models::Mode;
use serde::{Deserialize, Serialize};

/// Represents the different actions that can be performed with WebSocket requests.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum RequestActions {
    /// Subscribe to instrument tokens.
    Subscribe,
    /// Unsubscribe from instrument tokens.
    Unsubscribe,
    /// Set the mode for streaming data.
    Mode,
}

/// Represents the data associated with WebSocket requests.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum RequestData {
    /// A list of instrument tokens.
    InstrumentTokens(Vec<u32>),
    /// A mode and a list of instrument tokens.
    InstrumentTokensWithMode(Mode, Vec<u32>),
}

/// Represents the structure of a WebSocket request.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TickerRequest {
    /// The action to be performed.
    a: RequestActions,
    /// The data associated with the request action.
    v: RequestData,
}

impl TickerRequest {
    /// Creates a new `TickerRequest`.
    fn new(action: RequestActions, value: RequestData) -> TickerRequest {
        TickerRequest { a: action, v: value }
    }

    /// Creates a `TickerRequest` to subscribe to a list of instrument tokens.
    pub fn subscribe(instrument_tokens: Vec<u32>) -> TickerRequest {
        TickerRequest::new(
            RequestActions::Subscribe,
            RequestData::InstrumentTokens(instrument_tokens),
        )
    }

    /// Creates a `TickerRequest` to subscribe to a list of instrument tokens with
    /// a specified mode.
    pub fn subscribe_with_mode(instrument_tokens: Vec<u32>, mode: Mode) -> TickerRequest {
        TickerRequest::new(
            RequestActions::Mode,
            RequestData::InstrumentTokensWithMode(mode, instrument_tokens),
        )
    }

    /// Creates a `TickerRequest` to unsubscribe from a list of instrument tokens.
    pub fn unsubscribe(instrument_tokens: Vec<u32>) -> TickerRequest {
        TickerRequest::new(
            RequestActions::Unsubscribe,
            RequestData::InstrumentTokens(instrument_tokens),
        )
    }
}

impl ToString for TickerRequest {
    /// Converts the `TickerRequest` to a JSON string.
    ///
    /// This method serializes the `TickerRequest` into a JSON string representation.
    fn to_string(&self) -> String {
        serde_json::to_string(self).expect("failed to serialize TickerInput to JSON")
    }
}

