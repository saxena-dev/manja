//! Legacy HTTP configuration traits. Compiled with the `http` feature.
//!
//! [`KiteConfig`] is implemented by [`Config`](crate::kite::connect::config::Config):
//! it builds endpoint URLs from the API base. [`KiteAuth`] adds a Kite
//! `Authorization` header to a `reqwest` header map. The HTTP client builds
//! its own headers from a validated credential snapshot and needs neither;
//! they remain for existing callers.
//!
use reqwest::header::{HeaderMap, HeaderValue};

/// URL construction for Kite Connect HTTP endpoints.
pub trait KiteConfig: Send {
    /// The full URL of `path`, which starts with a slash.
    fn url(&self, path: &str) -> String;

    /// The API base URL.
    fn api_base(&self) -> &str;
}

/// Trait for adding the `Authorization` header to HTTP requests as required by Kite Connect.
///
/// This trait defines a method for adding the `Authorization` header to a `HeaderMap`
/// with the `api_key:access_token` combination.
///
pub trait KiteAuth {
    /// Adds the `Authorization` header to the `HeaderMap`.
    ///
    /// This method constructs the header as specified in the official Kite Connect
    /// [documentation](https://kite.trade/docs/connect/v3/user/#signing-requests) for signing HTTP requests.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The API key for Kite Connect.
    /// * `access_token` - The access token obtained after login.
    ///
    fn add_auth_header(&mut self, api_key: String, access_token: String);
}

impl KiteAuth for HeaderMap {
    fn add_auth_header(&mut self, api_key: String, access_token: String) {
        if let Ok(value) =
            HeaderValue::from_str(format!("token {}:{}", api_key, access_token).as_ref())
        {
            // Once the authentication is complete, all requests should be
            // signed with the HTTP `Authorization` header with `token` as
            // the authorization scheme, followed by a space, and then the
            // `api_key:access_token` combination.
            self.insert("Authorization", value);
        }
    }
}
