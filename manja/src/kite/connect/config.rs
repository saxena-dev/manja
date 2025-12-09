//! Configuration for asynchronous HTTP client.
//!
//! This module provides configurations for the async HTTP client, including default
//! URLs, environment variable handling, and header management for API requests.
//!
//! # Environment variables:
//!
//! The following environment variables can be specified to override the default values:
//!
//! - `KITECONNECT_API_BASE`: The base URL for Kite Connect API.
//! - `KITECONNECT_API_LOGIN`: The login URL for Kite Connect API.
//! - `KITECONNECT_API_REDIRECT`: The redirect URL for Kite Connect API.
//!
use reqwest::header::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret, Secret};

use crate::kite::connect::credentials::KiteCredentials;
use crate::kite::traits::{KiteAuth, KiteConfig};

/// Default v3 API base url.
///
pub const KITECONNECT_API_BASE: &str = "https://api.kite.trade";

/// Default v3 API login url.
///
pub const KITECONNECT_API_LOGIN: &str = "https://kite.trade/connect/login";

/// Default KiteConnect redirect url.
///
pub const KITECONNECT_API_REDIRECT: &str = "https://127.0.0.1/kite-redirect?";

/// Represents the KiteConnect client configurations.
///
/// This struct holds the API base URL, login URL, redirect URL, and user credentials.
///
#[derive(Clone, Debug)]
pub struct Config {
    /// Base URL for the KiteConnect API.
    api_base: String,
    /// Login URL for the KiteConnect API.
    api_login: String,
    /// Redirect URL for the KiteConnect API.
    api_redirect: String,
    /// User credentials for KiteConnect API.
    credentials: KiteCredentials,
}

impl Default for Config {
    /// Default implementation of `KiteConfig` picks up values from environment variables.
    ///
    /// If the environment variables are not set, it falls back to the default values.
    ///
    fn default() -> Self {
        Self {
            api_base: std::env::var("KITECONNECT_API_BASE")
                .unwrap_or_else(|_| KITECONNECT_API_BASE.to_string())
                .into(),
            api_login: std::env::var("KITECONNECT_API_LOGIN")
                .unwrap_or_else(|_| KITECONNECT_API_LOGIN.to_string())
                .into(),
            api_redirect: std::env::var("KITECONNECT_API_REDIRECT")
                .unwrap_or_else(|_| KITECONNECT_API_REDIRECT.to_string())
                .into(),
            credentials: KiteCredentials::load_from_env(),
        }
    }
}

impl KiteConfig for Config {
    /// Returns the HTTP headers required for API requests.
    ///
    /// If an access token is provided, it is included in the headers.
    ///
    /// # Arguments
    ///
    /// * `access_token` - An optional access token for authentication.
    ///
    /// # Returns
    ///
    /// A `HeaderMap` containing the necessary headers for API requests.
    ///
    fn headers(&self, access_token: Option<Secret<String>>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        // NOTE: `KiteConfig` currently points to v3.0 of Kite Connect API.
        // This could be made configurable if Zerodha announces any breaking changes
        // to the API.
        headers.insert("X-Kite-Version", HeaderValue::from_static("3"));
        if let Some(access_token) = access_token {
            headers.add_auth_header(
                self.credentials.api_key().expose_secret().clone(),
                access_token.expose_secret().clone(),
            )
        }
        headers
    }

    /// Constructs a URL endpoint given a path.
    ///
    /// NOTE: The `path` should have a leading backslash.
    ///
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    /// Returns the base URL for the KiteConnect API.
    ///
    fn api_base(&self) -> &str {
        self.api_base.as_str()
    }

    /// Returns the login URL for the KiteConnect API.
    ///
    fn api_login(&self) -> &str {
        self.api_login.as_str()
    }

    /// Returns the redirect URL for the KiteConnect API.
    ///
    fn api_redirect(&self) -> &str {
        self.api_redirect.as_str()
    }

    /// Returns the user credentials for the KiteConnect API.
    fn credentials(&self) -> &KiteCredentials {
        &self.credentials
    }
}

impl Config {
    /// Constructs a `Config` from individual parts.
    ///
    /// # Arguments
    ///
    /// * `api_base` - The base URL for the KiteConnect API.
    /// * `api_login` - The login URL for the KiteConnect API.
    /// * `api_redirect` - The redirect URL for the KiteConnect API.
    /// * `credentials` - The user credentials for the KiteConnect API.
    ///
    /// # Returns
    ///
    /// A `Config` instance.
    ///
    pub fn from_parts<InS>(
        api_base: InS,
        api_login: InS,
        api_redirect: InS,
        credentials: KiteCredentials,
    ) -> Self
    where
        InS: Into<String>,
    {
        Self {
            api_base: api_base.into(),
            api_login: api_login.into(),
            api_redirect: api_redirect.into(),
            credentials,
        }
    }
}
