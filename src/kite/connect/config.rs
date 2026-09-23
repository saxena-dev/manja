//! Configuration for the asynchronous HTTP client.
//!
//! [`Config`] holds the API base URL plus, until the `login` module is
//! removed, the legacy login and redirect URLs and the legacy
//! [`KiteCredentials`] bundle that module reads.
//!
//! Configuration is always explicit. Nothing here reads environment
//! variables: [`Config::default`] is the documented production endpoint set
//! with empty legacy login material, and [`Config::from_parts`] takes every
//! value from the caller.
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
/// This struct holds the API base URL, login URL, redirect URL, and the legacy
/// login credentials. `Debug` redacts the credentials.
///
#[derive(Clone, Debug)]
pub struct Config {
    /// Base URL for the KiteConnect API.
    api_base: String,
    /// Login URL for the KiteConnect API.
    api_login: String,
    /// Redirect URL for the KiteConnect API.
    api_redirect: String,
    /// Legacy login credentials, read only by the `login` module.
    credentials: KiteCredentials,
}

impl Default for Config {
    /// The documented production URLs, with empty legacy login credentials.
    ///
    /// No environment variable is consulted.
    fn default() -> Self {
        Self {
            api_base: KITECONNECT_API_BASE.to_string(),
            api_login: KITECONNECT_API_LOGIN.to_string(),
            api_redirect: KITECONNECT_API_REDIRECT.to_string(),
            credentials: KiteCredentials::new("", "", "", "", ""),
        }
    }
}

impl KiteConfig for Config {
    /// Returns the legacy HTTP headers: `X-Kite-Version: 3` and, when an access
    /// token is given, an `Authorization` header.
    ///
    /// Retained for the legacy `KiteConfig` trait; the HTTP client builds its
    /// headers from a validated `Credentials` snapshot instead.
    fn headers(&self, access_token: Option<Secret<String>>) -> HeaderMap {
        let mut headers = HeaderMap::new();
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
    /// NOTE: The `path` should have a leading slash.
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

    /// Returns the legacy login credentials.
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
    /// * `credentials` - The legacy login credentials.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_reads_no_environment() {
        std::env::set_var("KITECONNECT_API_BASE", "http://sentinel.invalid");
        let config = Config::default();
        assert_eq!(config.api_base(), KITECONNECT_API_BASE);
        assert_eq!(config.api_login(), KITECONNECT_API_LOGIN);
        assert_eq!(config.api_redirect(), KITECONNECT_API_REDIRECT);
    }

    #[test]
    fn debug_redacts_legacy_credentials() {
        let config = Config::from_parts(
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
            KiteCredentials::new("k", "SENTINEL", "u", "SENTINEL", "SENTINEL"),
        );
        assert!(!format!("{config:?}").contains("SENTINEL"));
    }
}
