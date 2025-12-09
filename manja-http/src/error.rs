//! Error types for the `manja-http` crate.
//!
//! This module defines a lightweight error type used by the HTTP client and
//! API groups. It focuses on transport- and serialization-related failures
//! and reuses the shared API-layer error descriptions from `manja-core`.

use std::io;

use manja_core::error::KiteApiError;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

/// Result alias used throughout the `manja-http` crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Error type for HTTP and JSON/IO failures in `manja-http`.
#[derive(Debug, Error)]
pub enum Error {
    /// Represents errors returned by the Kite Connect HTTP API.
    #[error("KiteConnect API error: {0}")]
    KiteApi(KiteApiError),

    /// Represents errors related to invalid HTTP headers.
    #[error("Invalid header value: {0}")]
    InvalidHeaderValue(#[from] InvalidHeaderValue),

    /// Represents errors that occur during JSON deserialization of Kite API
    /// responses.
    #[error("failed to deserialize Kite API response: {0}")]
    Json(#[from] serde_json::Error),

    /// Represents general I/O errors.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// Represents HTTP client errors.
    #[error("HTTP error: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// Represents internal errors within the `manja-http` crate.
    #[error("internal manja-http error: {0}")]
    Internal(String),
}

impl From<&str> for Error {
    fn from(value: &str) -> Self {
        Error::Internal(value.to_string())
    }
}

impl From<KiteApiError> for Error {
    fn from(err: KiteApiError) -> Self {
        Error::KiteApi(err)
    }
}

/// Utility function to map deserialization errors to [`Error`] while logging
/// the JSON string that caused the error.
pub(crate) fn map_deserialization_error(e: serde_json::Error, json_str: &str) -> Error {
    tracing::error!("failed deserialization of: {}", json_str);
    Error::Json(e)
}
