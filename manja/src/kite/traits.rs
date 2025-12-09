//! Trait definitions for implementing custom types that work with `manja`.
//!
//! This module defines various traits used for configuring and interacting with
//! Kite Connect API via `manja`. These traits provide the necessary interfaces
//! for managing API calls, handling authentication, and executing login flows.
//!
use std::future::Future;
use std::pin::Pin;

use reqwest::header::{HeaderMap, HeaderValue};
use secrecy::Secret;

use crate::kite::connect::credentials::KiteCredentials;
use crate::kite::error::Result;

/// Configuration details required for making API calls to Kite Connect.
///
/// This trait must be implemented by any configuration struct used by the `HTTPClient`
/// for making REST API calls to Kite Connect. It includes methods for constructing headers,
/// generating URLs, and accessing credentials.
///
pub trait KiteConfig: Send {
    /// Generates the headers required for making API requests.
    ///
    /// # Arguments
    ///
    /// * `access_token` - An optional secret containing the access token for authorization.
    ///
    /// # Returns
    ///
    /// A `HeaderMap` containing the necessary headers.
    ///
    fn headers(&self, access_token: Option<Secret<String>>) -> HeaderMap;

    /// Constructs the full URL for a given API path.
    ///
    /// # Arguments
    ///
    /// * `path` - The API endpoint path.
    ///
    /// # Returns
    ///
    /// A `String` containing the full URL.
    ///
    fn url(&self, path: &str) -> String;

    /// Returns the base URL for Kite Connect API.
    ///
    /// # Returns
    ///
    /// A string slice containing the base URL.
    ///
    fn api_base(&self) -> &str;

    /// Returns the URL for Kite Connect login page.
    ///
    /// # Returns
    ///
    /// A string slice containing the login URL.
    ///
    fn api_login(&self) -> &str;

    /// Returns the redirect URL after a successful login.
    ///
    /// # Returns
    ///
    /// A string slice containing the redirect URL.
    ///
    fn api_redirect(&self) -> &str;

    /// Provides the credentials required for authentication.
    ///
    /// # Returns
    ///
    /// A reference to `KiteCredentials` containing the necessary credentials.
    ///
    fn credentials(&self) -> &KiteCredentials;
}

/// Trait for managing the login flow for Kite Connect.
///
/// This trait defines a method for generating a request token by executing
/// an asynchronous function that handles the login flow.
///
pub trait KiteLoginFlow {
    /// Generates a request token by executing the provided asynchronous function.
    ///
    /// # Arguments
    ///
    /// * `f` - A closure that takes a boxed `KiteConfig` and returns a future
    ///   resolving to a `Result<String>`.
    ///
    /// # Returns
    ///
    /// A pinned box containing a future that resolves to a `Result<String>`.
    ///
    fn gen_request_token<F, Fut>(
        &self,
        f: F,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send>>
    where
        F: Fn(Box<dyn KiteConfig>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<String>> + Send + 'static;
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
        ()
    }
}
