//! Core, transport-agnostic traits shared across the `manja` workspace.
//!
//! This module defines small, composable traits that describe configuration,
//! credentials, authentication formatting, and login flows for Kite Connect.
//! The traits are intentionally non-IO and avoid tying to any specific HTTP
//! client, async runtime, or WebDriver implementation.

use std::future::Future;

use secrecy::Secret;

/// Describes the base/api/login endpoints used to talk to Kite Connect.
pub trait CoreApiEndpoints {
    /// Base URL for the Kite Connect HTTP API.
    fn api_base(&self) -> &str;

    /// Login URL for initiating the Kite Connect login flow.
    fn api_login(&self) -> &str;

    /// Redirect URL used after a successful login.
    fn api_redirect(&self) -> &str;
}

/// Describes the secret credentials required for interacting with Kite Connect.
///
/// This trait is transport-agnostic: it does not prescribe how these secrets are
/// stored, loaded, or used, only that they can be accessed when needed.
pub trait CoreCredentials {
    /// API key used to identify the client application.
    fn api_key(&self) -> Secret<String>;

    /// API secret associated with the API key.
    fn api_secret(&self) -> Secret<String>;

    /// User ID for the the Kite account.
    fn user_id(&self) -> Secret<String>;

    /// User password for the Kite account.
    fn user_password(&self) -> Secret<String>;

    /// TOTP key used for 2FA-based login flows.
    fn totp_key(&self) -> Secret<String>;
}

/// Composite configuration trait that combines endpoints and credentials.
///
/// Concrete crates are free to implement the underlying traits directly; this
/// trait serves as a convenient bound when both pieces are required.
pub trait CoreConfig: CoreApiEndpoints + CoreCredentials {}

impl<T> CoreConfig for T where T: CoreApiEndpoints + CoreCredentials {}

/// Describes how to build an authorization value (e.g. HTTP header value)
/// from an API key and access token.
///
/// This stays transport-agnostic by returning a string token that higher-layer
/// crates can inject into their own header or request types.
pub trait CoreAuth {
    /// Build an authorization value from the given API key and access token.
    fn build_auth_value(&self, api_key: &str, access_token: &str) -> String;
}

/// Default implementation of Kite's `token {api_key}:{access_token}` scheme.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultKiteAuth;

impl CoreAuth for DefaultKiteAuth {
    fn build_auth_value(&self, api_key: &str, access_token: &str) -> String {
        format!("token {}:{}", api_key, access_token)
    }
}

/// Core abstraction for login flows that can generate a request token given
/// some configuration.
///
/// This trait deliberately avoids choosing a specific async runtime. Implementors
/// are free to use `tokio`, WebDriver clients, or other async mechanisms in
/// their concrete code.
pub trait CoreLoginFlow<C> {
    /// Error type produced when generating a request token fails.
    type Error;

    /// Future type returned by [`CoreLoginFlow::gen_request_token`].
    ///
    /// The future must be `Send` so that it can be used across async executors
    /// if needed.
    type Fut<'a>: Future<Output = Result<String, Self::Error>> + Send + 'a
    where
        C: 'a,
        Self: 'a;

    /// Generate a request token using the provided configuration.
    fn gen_request_token<'a>(&'a self, config: &'a C) -> Self::Fut<'a>;
}

