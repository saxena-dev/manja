//! Core API-level error description types.
//!
//! This module defines the transport-agnostic error types that describe
//! failures returned by the Kite Connect HTTP API. These types are shared
//! across higher-level crates such as `manja`, `manja-http`, and others.

use std::fmt;

use serde::Deserialize;

/// Represents an error returned by the Kite Connect API.
///
/// This structure captures details about an error response from Kite Connect
/// API, including the endpoint that was accessed, the HTTP status code, an
/// optional error message, and the type of error as represented by the
/// [`KiteApiException`] enum.
#[derive(Debug, Deserialize)]
pub struct KiteApiError {
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
/// Connect API. It covers a wide range of scenarios, such as session token
/// issues, user account problems, order-related errors, network issues, and
/// more.
#[derive(Debug, Deserialize)]
pub enum KiteApiException {
    /// Indicates the expiry or invalidation of an authenticated session.
    ///
    /// Preceded by a 403 header, this indicates the expiry or invalidation of
    /// an authenticated session. This can be caused by the user logging out,
    /// a natural expiry, or the user logging into another Kite instance. When
    /// you encounter this error, you should clear the user's session and
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
    /// This error indicates that the KiteConnect API has been updated with a
    /// new `error_type`.
    DeserializationException(String),
}

impl KiteApiException {
    /// Returns the short identifier for this exception as it appears in Kite
    /// API responses.
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
            _ => KiteApiException::DeserializationException(s.to_string()),
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
            KiteApiException::GeneralException => {
                write!(f, "GeneralException: represents an unclassified error")
            }
            KiteApiException::DeserializationException(path) => write!(
                f,
                "DeserializationException: represents a JSON deserialization error of a response from a KiteConnect API endpoint (`{}`).",
                path
            ),
        }
    }
}

