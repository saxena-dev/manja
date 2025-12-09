//! Request types for WebSocket API.
//!
//! This module defines the structures and methods for creating and managing
//! WebSocket requests to interact with Kite Connect streaming API. It includes
//! enums for request actions and data, and provides methods to subscribe,
//! unsubscribe, and set modes for instrument tokens.
//!
use crate::kite::ticker::models::Mode;

use serde::{Deserialize, Serialize};

/// Represents the different actions that can be performed with WebSocket requests.
///
/// The actions include subscribing to instrument tokens, unsubscribing from them,
/// and setting the mode for streaming data.
///
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
///
/// This enum can either hold a vector of instrument tokens or a tuple of mode
/// and instrument tokens.
///
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum RequestData {
    /// A list of instrument tokens.
    InstrumentTokens(Vec<u32>),
    /// A mode and a list of instrument tokens.
    InstrumentTokensWithMode(Mode, Vec<u32>),
}

/// Represents the structure of a WebSocket request.
///
/// This struct combines a request action and the associated data, providing a unified
/// structure for all WebSocket requests.
///
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TickerRequest {
    /// The action to be performed.
    a: RequestActions,
    /// The data associated with the request action.
    v: RequestData,
}

impl TickerRequest {
    /// Creates a new `TickerRequest`.
    ///
    /// # Arguments
    ///
    /// * `action` - The action to be performed (subscribe, unsubscribe, set mode).
    /// * `value` - The data associated with the action.
    ///
    /// # Returns
    ///
    /// A new `TickerRequest` instance.
    ///
    fn new(action: RequestActions, value: RequestData) -> TickerRequest {
        TickerRequest {
            a: action,
            v: value,
        }
    }

    /// Creates a `TickerRequest` to subscribe to a list of instrument tokens.
    ///
    /// # Arguments
    ///
    /// * `instrument_tokens` - A vector of instrument tokens to subscribe to.
    ///
    /// # Returns
    ///
    /// A new `TickerRequest` instance for subscribing to the provided instrument tokens.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let request = TickerRequest::subscribe(vec![12345, 67890]);
    /// ```
    ///
    pub fn subscribe(instrument_tokens: Vec<u32>) -> TickerRequest {
        TickerRequest::new(
            RequestActions::Subscribe,
            RequestData::InstrumentTokens(instrument_tokens),
        )
    }

    /// Creates a `TickerRequest` to subscribe to a list of instrument tokens with
    /// a specified mode.
    ///
    /// # Arguments
    ///
    /// * `instrument_tokens` - A vector of instrument tokens to subscribe to.
    /// * `mode` - The mode for streaming data.
    ///
    /// # Returns
    ///
    /// A new `TickerRequest` instance for subscribing to the provided instrument
    /// tokens with the specified mode.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let request = TickerRequest::subscribe_with_mode(vec![12345, 67890], Mode::Full);
    /// ```
    ///
    pub fn subscribe_with_mode(instrument_tokens: Vec<u32>, mode: Mode) -> TickerRequest {
        TickerRequest::new(
            RequestActions::Mode,
            RequestData::InstrumentTokensWithMode(mode, instrument_tokens),
        )
    }

    /// Creates a `TickerRequest` to unsubscribe from a list of instrument tokens.
    ///
    /// # Arguments
    ///
    /// * `instrument_tokens` - A vector of instrument tokens to unsubscribe from.
    ///
    /// # Returns
    ///
    /// A new `TickerRequest` instance for unsubscribing from the provided instrument tokens.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let request = TickerRequest::unsubscribe(vec![12345, 67890]);
    /// ```
    ///
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
    ///
    /// # Returns
    ///
    /// A `String` containing the JSON representation of the `TickerRequest`.
    ///
    /// # Panics
    ///
    /// This method will panic if serialization fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let request = TickerRequest::subscribe(vec![12345, 67890]);
    /// let json = request.to_string();
    /// println!("JSON: {}", json);
    /// ```
    ///
    fn to_string(&self) -> String {
        serde_json::to_string(self).expect("failed to serialize TickerInput to JSON")
    }
}
