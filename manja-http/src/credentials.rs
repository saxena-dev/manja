//! User credential type for accessing Kite Connect API.
//!
//! This module provides the `KiteCredentials` struct for securely handling
//! user credentials required for accessing the Kite Connect HTTP APIs.

use secrecy::Secret;

/// Represents Kite Connect credentials.
///
/// This struct securely stores the credentials required to access Kite
/// Connect APIs. When `KiteCredentials` is dropped, its contents are zeroed
/// in memory to help prevent leakage.
#[derive(Clone, Debug)]
pub struct KiteCredentials {
    api_key: Secret<String>,
    api_secret: Secret<String>,
    user_id: Secret<String>,
    user_pwd: Secret<String>,
    totp_key: Secret<String>,
}

impl Default for KiteCredentials {
    /// Creates `KiteCredentials` using values from environment variables.
    ///
    /// If the environment variables are not set, the corresponding fields
    /// will be empty strings.
    fn default() -> Self {
        // Default to loading credentials from environment variables
        Self::load_from_env()
    }
}

impl KiteCredentials {
    /// Creates `KiteCredentials`.
    ///
    /// Intended to be used from a custom credentials provider implementation.
    /// It is **not** safe to hardcode credentials in your application.
    pub fn new<InS>(
        api_key: InS,
        api_secret: InS,
        user_id: InS,
        user_pwd: InS,
        totp_key: InS,
    ) -> Self
    where
        InS: Into<String>,
    {
        KiteCredentials {
            api_key: Secret::new(api_key.into()),
            api_secret: Secret::new(api_secret.into()),
            user_id: Secret::new(user_id.into()),
            user_pwd: Secret::new(user_pwd.into()),
            totp_key: Secret::new(totp_key.into()),
        }
    }

    /// Loads credentials from environment variables.
    pub fn load_from_env() -> Self {
        Self {
            api_key: std::env::var("KITECONNECT_API_KEY")
                .unwrap_or_else(|_| "".to_string())
                .into(),
            api_secret: std::env::var("KITECONNECT_API_SECRET")
                .unwrap_or_else(|_| "".to_string())
                .into(),
            user_id: std::env::var("KITECONNECT_USER_ID")
                .unwrap_or_else(|_| "".to_string())
                .into(),
            user_pwd: std::env::var("KITECONNECT_PASSWORD")
                .unwrap_or_else(|_| "".to_string())
                .into(),
            totp_key: std::env::var("KITECONNECT_TOTP_KEY")
                .unwrap_or_else(|_| "".to_string())
                .into(),
        }
    }

    /// Returns the API key.
    pub fn api_key(&self) -> Secret<String> {
        self.api_key.clone()
    }

    /// Returns the API secret.
    pub fn api_secret(&self) -> Secret<String> {
        self.api_secret.clone()
    }

    /// Returns the user ID.
    pub fn user_id(&self) -> Secret<String> {
        self.user_id.clone()
    }

    /// Returns the user password.
    pub fn user_pwd(&self) -> Secret<String> {
        self.user_pwd.clone()
    }

    /// Returns the TOTP key.
    pub fn totp_key(&self) -> Secret<String> {
        self.totp_key.clone()
    }
}

