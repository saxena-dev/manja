//! Streaming mode enums for WebSocket API.
//!
//! This module defines the `Mode` enum, which represents the different modes
//! in which data packets can be streamed from Kite Connect WebSocket API. The
//! modes determine the type and detail level of the streaming data.
//!
use serde::{Deserialize, Serialize};

/// Represents the different modes in which packets are streamed.
///
/// The `Mode` enum is used to specify the type of data packets received from
/// the WebSocket streaming API. It supports conversion from `usize` values that
/// represent the packet sizes for different modes.
///
/// # Variants
///
/// - `Full`: Represents the mode where full data packets are streamed.
/// - `Quote`: Represents the mode where quote data packets are streamed. This is the default mode.
/// - `LTP`: Represents the mode where only the last traded price (LTP) data packets are streamed.
///
#[derive(Debug, Default, Clone, Eq, Hash, Deserialize, Serialize, PartialEq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Full data packets are streamed.
    Full,
    /// Quote data packets are streamed. This is the default mode.
    #[default]
    Quote,
    /// Only the last traded price (LTP) data packets are streamed.
    LTP,
}

impl Mode {
    /// The fixed order in which mode requests are sent: LTP, quote, full
    /// (`docs/contract.md` §2.6).
    pub const WIRE_ORDER: [Mode; 3] = [Mode::LTP, Mode::Quote, Mode::Full];

    /// The wire value: `ltp`, `quote` or `full`
    /// (`kite:websocket.md:51-57`).
    pub const fn as_wire(&self) -> &'static str {
        match self {
            Mode::LTP => "ltp",
            Mode::Quote => "quote",
            Mode::Full => "full",
        }
    }
}

impl TryFrom<usize> for Mode {
    type Error = String;

    // Attempts to convert a `usize` value to a `Mode`.
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            8 => Ok(Self::LTP),
            44 => Ok(Self::Quote),
            184 => Ok(Self::Full),
            _ => Err(format!("Invalid packet size: {}", value)),
        }
    }
}
