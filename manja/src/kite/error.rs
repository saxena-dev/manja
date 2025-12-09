//! Error types.
//!
//! This module defines custom error types and handling mechanisms for the
//! `manja` crate. The primary error type is [`ManjaError`], which consolidates
//! all possible errors that can occur when interacting with the Kite Connect
//! API and related services.
//!
//! Transport-agnostic API error description types such as
//! [`KiteApiError`](manja_core::error::KiteApiError) and
//! [`KiteApiException`](manja_core::error::KiteApiException) live in the
//! `manja-core` crate and are re-used here.

use std::env::VarError;

#[cfg(feature = "webdriver-login")]
use fantoccini::error::{CmdError, NewSessionError};
use manja_core::error::KiteApiError;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

/// A `Result` alias where the `Err` case is [`ManjaError`].
pub type Result<T> = std::result::Result<T, ManjaError>;

/// An enumeration of all possible errors that may occur when using the `manja`
/// crate.
///
/// This enum provides a consolidated view of error types, including those
/// originating from external crates like `reqwest` and `fantoccini`. Each
/// variant represents a specific failure scenario that can be encountered when
/// using the SDK.
#[derive(Debug, Error)]
pub enum ManjaError {
    /// Represents errors returned by Kite Connect API.
    #[error("KiteConnect API error: {0}")]
    KiteApiError(KiteApiError),

    /// Represents errors related to missing or invalid environment variables.
    #[error("Environment variable error: {0}")]
    EnvVarError(#[from] VarError),

    /// Represents errors related to invalid HTTP headers.
    #[error("Invalid header value: {0}")]
    InvalidHeaderValueError(#[from] InvalidHeaderValue),

    #[cfg(feature = "webdriver-login")]
    /// Represents errors related to starting a new WebDriver session.
    #[error("WebDriver new session error: {0}")]
    WebDriverNewSessionError(#[from] NewSessionError),

    #[cfg(feature = "webdriver-login")]
    /// Represents general WebDriver errors.
    #[error("WebDriver error: {0}")]
    WebDriverError(#[from] CmdError),

    /// Represents errors that occur during JSON deserialization of Kite API
    /// responses.
    #[error("failed to deserialize Kite API response: {0}")]
    JSONDeserialize(#[from] serde_json::Error),

    /// Represents general I/O errors.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Represents HTTP request errors.
    #[error("HTTP error: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// Represents errors related to Time-based One-Time Password (TOTP)
    /// generation or validation.
    #[error("TOTP error: {0}")]
    TotpError(String),

    /// Represents internal errors within the `manja` crate.
    #[error("internal manja error: {0}")]
    Internal(String),
}

impl From<&str> for ManjaError {
    fn from(value: &str) -> Self {
        ManjaError::Internal(value.to_string())
    }
}

/// Re-export the core API-level exception type so existing imports from
/// `crate::kite::error::KiteApiException` continue to work.
pub use manja_core::error::KiteApiException;

/// Utility function to map deserialization errors to [`ManjaError`] while
/// logging the JSON string that caused the error.
///
/// This is useful for debugging deserialization issues by capturing and
/// logging the raw JSON string that failed to deserialize. It returns a
/// [`ManjaError::JSONDeserialize`] variant with the captured
/// [`serde_json::Error`].
pub(crate) fn map_deserialization_error(e: serde_json::Error, json_str: &str) -> ManjaError {
    tracing::error!("failed deserialization of: {}", json_str);
    ManjaError::JSONDeserialize(e)
}
