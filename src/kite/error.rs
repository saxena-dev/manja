//! Error types.
//! 
//! This module defines custom error types and handling mechanisms for the `manja` crate.
//! It includes various error types that represent different failure scenarios 
//! when interacting with Kite Connect API and other related services. 
//!
//! The primary error type is `ManjaError`, which consolidates all possible errors 
//! that can occur during the execution of the client code. This module also provides 
//! convenient error mapping from other crates like `reqwest`, `serde`, and `fantoccini`.
//! 
use std::env::VarError;
use std::fmt;

#[cfg(feature = "webdriver-login")]
use fantoccini::error::{CmdError, NewSessionError};
use reqwest::header::InvalidHeaderValue;
use serde::Deserialize;


/// A `Result` alias where the `Err` case is `manja::kite::ManjaError`.
pub type Result<T> = std::result::Result<T, ManjaError>;

/// An enumeration of all possible errors that may occur when using the `manja` crate.
///
/// This enum provides a consolidated view of all error types, including those 
/// originating from external crates like `reqwest` and `fantoccini`. Each variant 
/// represents a specific type of error that can be encountered during the operation 
/// of an API client provided by `manja`.
/// 
#[derive(Debug, thiserror::Error)]
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

    /// Represents errors that occur during JSON deserialization of Kite API responses.
    #[error("failed to deserialize Kite API response: {0}")]
    JSONDeserialize(#[from] serde_json::Error),

    /// Represents general I/O errors.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Represents HTTP request errors.
    #[error("HTTP error: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// Represents errors related to Time-based One-Time Password (TOTP) generation or validation.
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

/// Represents an error returned by Kite Connect API.
///
/// This structure captures details about an error response from Kite Connect API,
/// including the endpoint that was accessed, the HTTP status code, an optional 
/// error message, and the type of error as represented by the `KiteApiException` enum.
/// 
#[derive(Debug, Deserialize)]
pub(crate) struct KiteApiError {
    pub endpoint: String,
    pub status_code: u16,
    pub message: Option<String>,
    pub error_type: KiteApiException,
}

impl fmt::Display for KiteApiError { 
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            Some(msg) if !msg.is_empty() => write!(
                f,
                "HTTP {} {} at {}: {}",
                self.status_code,
                self.error_type.as_str(),
                self.endpoint,
                msg
            ),
            _ => write!(
                f,
                "HTTP {} {} at {}",
                self.status_code,
                self.error_type.as_str(),
                self.endpoint
            ),
        }
    }
}



/// Enum representing various types of errors that can occur while interacting 
/// with Kite Connect API.
///
/// This enum categorizes different error types that might be returned by Kite 
/// Connect API. It covers a wide range of scenarios, such as session token issues, 
/// user account problems, order-related errors, network issues, and more.
///
#[derive(Debug, Deserialize)]
pub enum KiteApiException {
    /// Indicates the expiry or invalidation of an authenticated session.
    /// 
    /// Preceded by a 403 header, this indicates the expiry or invalidation of
    /// an authenticated session. This can be caused by the user logging out,
    /// a natural expiry, or the user logging into another Kite instance.
    /// When you encounter this error, you should clear the user's session and
    /// re-initiate a login.
    TokenException,

    /// Represents user account related errors.
    UserException,

    /// Represents order related errors such as placement failures or a corrupt
    /// fetch.
    OrderException,

    /// Represents missing required fields or bad values for parameters.
    InputException,

    /// Represents insufficient funds required for order placement.
    MarginException,

    /// Represents insufficient holdings available to place a sell order for a
    /// specified instrument.
    HoldingException,

    /// Represents a network error where the API was unable to communicate
    /// with the Order Management System (OMS).
    NetworkException,

    /// Represents an internal system error where the API was unable to
    /// understand the response from the OMS to respond to a request.
    DataException,

    /// Represents an unclassified error. This should only happen rarely.
    GeneralException,

    /// Represents a deserialization error from a KiteConnect API response.
    /// This error indicates that the KiteConnect API has been updated with
    /// a new `error_type`.
    DeserializationException(String),
}

impl KiteApiException {
    /// Returns the short identifier for this exception as it appears in Kite API responses.
    pub fn as_str(&self) -> &str {
        match self {
            KiteApiException::TokenException => "TokenException",
            KiteApiException::UserException => "UserException",
            KiteApiException::OrderException => "OrderException",
            KiteApiException::InputException => "InputException",
            KiteApiException::MarginException => "MarginException",
            KiteApiException::HoldingException => "HoldingException",
            KiteApiException::NetworkException => "NetworkException",
            KiteApiException::DataException => "DataException",
            KiteApiException::GeneralException => "GeneralException",
            KiteApiException::DeserializationException(_) => "DeserializationException",
        }
    }
}


impl From<&str> for KiteApiException {
    fn from(s: &str) -> Self {
        match s {
            "TokenException" => KiteApiException::TokenException,
            "UserException" => KiteApiException::UserException,
            "OrderException" => KiteApiException::OrderException,
            "InputException" => KiteApiException::InputException,
            "MarginException" => KiteApiException::MarginException,
            "HoldingException" => KiteApiException::HoldingException,
            "NetworkException" => KiteApiException::NetworkException,
            "DataException" => KiteApiException::DataException,
            "GeneralException" => KiteApiException::GeneralException,
            _ => KiteApiException::DeserializationException(s.to_string()), // Handle unknown input
        }
    }
}


impl fmt::Display for KiteApiException {
    #[allow(deprecated)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            KiteApiException::TokenException => write!(
                f,
                "TokenException: indicates the expiry or invalidation of an authenticated session"
            ),
            KiteApiException::UserException => write!(
                f, 
                "UserException: represents user account related errors"
            ),
            KiteApiException::OrderException => write!(
                f, 
                "OrderException: represents order related errors such as placement failures or a corrupt fetch"
            ),
            KiteApiException::InputException => write!(
                f, 
                "InputException: represents missing required fields or bad values for parameters"
            ),
            KiteApiException::MarginException => write!(
                f, 
                "MarginException: represents insufficient funds required for order placement"
            ),
            KiteApiException::HoldingException => write!(
                f, 
                "HoldingException: represents insufficient holdings available to place a sell order for a specified instrument"
            ),
            KiteApiException::NetworkException => write!(
                f, 
                "NetworkException: represents a network error where the API was unable to communicate with the Order Management System (OMS)"
            ),
            KiteApiException::DataException => write!(
                f, 
                "DataException: represents an internal system error where the API was unable to understand the response from the OMS to respond to a request"
            ),
            KiteApiException::GeneralException => write!(
                f, 
                "GeneralException: represents an unclassified error"
            ),
            KiteApiException::DeserializationException(path) => write!(
                f,
                "DeserializationException: represents a JSON deserialization error of a response from a KiteConnect API endpoint (`{}`).",
                path
            )
        }
    }
}

// Utility function to map deserialization errors to `ManjaError` while logging 
// the JSON string that caused the error.
//
// This function is useful for debugging deserialization issues by capturing and 
// logging the raw JSON string that failed to deserialize. It returns a 
// `ManjaError::JSONDeserialize` variant with the captured `serde_json::Error`.
// 
pub(crate) fn map_deserialization_error(e: serde_json::Error, json_str: &str) -> ManjaError {
    tracing::error!("failed deserialization of: {}", json_str);
    ManjaError::JSONDeserialize(e)
}
