//! Configuration for the asynchronous HTTP client.
//!
//! This module provides configuration for the HTTP client, including default
//! URLs, environment variable handling, and header management for API
//! requests.

use reqwest::header::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret, Secret};

use crate::credentials::KiteCredentials;
use manja_core::traits::{CoreApiEndpoints, CoreAuth, CoreCredentials, DefaultKiteAuth};

/// Default v3 API base url.
pub const KITECONNECT_API_BASE: &str = "https://api.kite.trade";

/// Default v3 API login url.
pub const KITECONNECT_API_LOGIN: &str = "https://kite.trade/connect/login";

/// Default KiteConnect redirect url.
pub const KITECONNECT_API_REDIRECT: &str = "https://127.0.0.1/kite-redirect?";

/// Represents the KiteConnect client configuration.
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
    /// Default implementation picks up values from environment variables.
    ///
    /// If the environment variables are not set, it falls back to the
    /// default values above.
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

impl Config {
    /// Constructs a `Config` from individual parts.
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

    /// Returns the HTTP headers required for API requests.
    ///
    /// If an access token is provided, it is included in the headers as an
    /// `Authorization` header using the default Kite auth scheme.
    pub fn headers(&self, access_token: Option<Secret<String>>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        // NOTE: `Config` currently points to v3.0 of Kite Connect API.
        headers.insert("X-Kite-Version", HeaderValue::from_static("3"));
        if let Some(access_token) = access_token {
            let api_key = self.credentials.api_key().expose_secret().clone();
            let access_token = access_token.expose_secret().clone();
            let auth_value = DefaultKiteAuth.build_auth_value(&api_key, &access_token);
            if let Ok(value) = HeaderValue::from_str(auth_value.as_ref()) {
                // Once the authentication is complete, all requests should be
                // signed with the HTTP `Authorization` header with `token` as
                // the authorization scheme, followed by a space, and then the
                // `api_key:access_token` combination.
                headers.insert("Authorization", value);
            }
        }
        headers
    }

    /// Constructs a URL endpoint given a path.
    ///
    /// NOTE: The `path` should have a leading slash.
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    /// Returns the user credentials for the KiteConnect API.
    pub fn credentials(&self) -> &KiteCredentials {
        &self.credentials
    }
}

impl CoreApiEndpoints for Config {
    fn api_base(&self) -> &str {
        self.api_base.as_str()
    }

    fn api_login(&self) -> &str {
        self.api_login.as_str()
    }

    fn api_redirect(&self) -> &str {
        self.api_redirect.as_str()
    }
}

impl CoreCredentials for Config {
    fn api_key(&self) -> Secret<String> {
        self.credentials.api_key()
    }

    fn api_secret(&self) -> Secret<String> {
        self.credentials.api_secret()
    }

    fn user_id(&self) -> Secret<String> {
        self.credentials.user_id()
    }

    fn user_password(&self) -> Secret<String> {
        self.credentials.user_pwd()
    }

    fn totp_key(&self) -> Secret<String> {
        self.credentials.totp_key()
    }
}
