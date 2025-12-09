use std::env::VarError;

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
    WebDriverNewSessionError(String),

    #[cfg(feature = "webdriver-login")]
    /// Represents general WebDriver errors.
    #[error("WebDriver error: {0}")]
    WebDriverError(String),

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

#[cfg(feature = "webdriver-login")]
impl From<manja_extras::login::LoginError> for ManjaError {
    fn from(value: manja_extras::login::LoginError) -> Self {
        use manja_extras::login::LoginError;

        match value {
            LoginError::EnvVarError(e) => ManjaError::EnvVarError(e),
            LoginError::WebDriverNewSessionError(e) => {
                ManjaError::WebDriverNewSessionError(e.to_string())
            }
            LoginError::WebDriverError(e) => ManjaError::WebDriverError(e.to_string()),
            LoginError::TotpError(msg) => ManjaError::TotpError(msg),
            LoginError::InvalidRedirectUrl(url) => ManjaError::Internal(format!(
                "cannot parse Kite redirect url - `{}`",
                url
            )),
            LoginError::Timeout => {
                ManjaError::Internal("Timed out waiting for redirect URL".to_string())
            }
            LoginError::Internal(msg) => ManjaError::Internal(msg),
        }
    }
}
