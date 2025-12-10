//! Error types for WebDriver-assisted login flows.

use std::env::VarError;

use fantoccini::error::{CmdError, NewSessionError};
use thiserror::Error;

/// Result alias for WebDriver login helpers.
pub type Result<T> = std::result::Result<T, LoginError>;

/// Error type produced by WebDriver/TOTP-based login flows.
///
/// This error is intentionally scoped to the login runtime so that the core
/// `manja` crates can remain free of WebDriver-specific dependencies.
#[derive(Debug, Error)]
pub enum LoginError {
    /// Represents errors related to missing or invalid environment variables.
    #[error("Environment variable error: {0}")]
    EnvVarError(#[from] VarError),

    /// Represents errors related to starting a new WebDriver session.
    #[error("WebDriver new session error: {0}")]
    WebDriverNewSessionError(#[from] NewSessionError),

    /// Represents general WebDriver errors.
    #[error("WebDriver error: {0}")]
    WebDriverError(#[from] CmdError),

    /// Represents errors related to Time-based One-Time Password (TOTP)
    /// generation or validation.
    #[error("TOTP error: {0}")]
    TotpError(String),

    /// Represents an invalid redirect URL configured for the login flow.
    #[error("invalid Kite redirect URL `{0}`")]
    InvalidRedirectUrl(String),

    /// Represents a timeout while waiting for the login redirect URL.
    #[error("timed out waiting for redirect URL")]
    Timeout,

    /// Represents internal errors within the login helpers.
    #[error("internal login error: {0}")]
    Internal(String),
}

impl From<&str> for LoginError {
    fn from(value: &str) -> Self {
        LoginError::Internal(value.to_string())
    }
}

