//! Session API group: `/session/`
//!
//! The [Session] API group provides endpoints for managing user sessions,
//! including the two-step login flow required to authenticate with Kite
//! Connect API.
//!
//! This module facilitates the following:
//!
//! 1. **Generating a request token**: Initiate the login flow by navigating to
//!    the Kite Connect login page with the `api_key`.
//! 2. **Exchanging the request token for an access token**: Use the `request_token`
//!    and a checksum to obtain an `access_token` for authenticated API requests.
//!
//! ![Login Flow Diagram](https://kite.trade/docs/connect/v3/images/kite-connect-flow.png)
//!
//! For detailed information, refer to the official KiteConnect API
//! [documentation](https://kite.trade/docs/connect/v3/user/#login-flow).
//!
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use secrecy::ExposeSecret;

use crate::kite::connect::{
    api::{create_backoff_policy, BackoffPolicy},
    client::HTTPClient,
    models::{KiteApiResponse, UserSession},
    utils::create_checksum,
};
use crate::kite::error::Result;
use crate::kite::traits::{KiteConfig, KiteLoginFlow};

/// User session related API endpoints for login and session management.
///
/// This struct handles operations related to user sessions, including login
/// and session management, by interfacing with the HTTP client.
///
/// For more details, refer to the official API [documentation](https://kite.trade/docs/connect/v3/user/#login-flow).
///
pub struct Session<'c> {
    /// A mutable reference to the HTTP client used for making API requests
    /// and storing a `UserSession` object after a successful login flow.
    pub client: &'c mut HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> KiteLoginFlow for Session<'c> {
    /// Generate a request token by executing an asynchronous function.
    ///
    /// This method accepts a closure that performs the asynchronous task of
    /// generating a request token using the provided `KiteConfig` configuration.
    ///
    /// # Arguments
    ///
    /// * `f` - A closure that takes a boxed `KiteConfig` and returns a future
    ///   that resolves to a `Result<String>`.
    ///
    /// # Returns
    ///
    /// A pinned box containing a future that resolves to a `Result<String>`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let request_token = session.gen_request_token(|config| async move {
    ///     // Implement the logic to generate the request token
    /// }).await?;
    /// ```
    ///
    fn gen_request_token<F, Fut>(
        &self,
        f: F,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send>>
    where
        F: Fn(Box<dyn KiteConfig>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<String>> + Send + 'static,
    {
        let config = Box::new(self.client.http_config().to_owned());
        Box::pin(async move { f(config).await })
    }
}

// --- [ impl Session ] ---

impl<'c> Session<'c> {
    /// Creates a new `Session` instance.
    ///
    /// # Arguments
    ///
    /// * `client` - A mutable reference to an `HTTPClient` instance.
    ///
    /// # Returns
    ///
    /// Returns a new `Session` instance containing the provided HTTP client.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut client = HTTPClient::new();
    /// let session = Session::new(&mut client);
    /// ```
    ///
    pub fn new(client: &'c mut HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Session` instance.
    ///
    /// # Arguments
    ///
    /// * `backoff` - A `BackoffPolicy` instance specifying the backoff policy.
    ///
    /// # Returns
    ///
    /// The `User` instance with the updated backoff policy.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Generates a session using a `request token`.
    ///
    /// This method completes the second step of the
    /// [login flow](https://kite.trade/docs/connect/v3/user/#login-flow),
    /// where a `request token` obtained from the initial login step is used
    /// to generate a valid session and obtain an `access_token`.
    ///
    /// # Arguments
    ///
    /// * `request_token` - The token received after the initial login step,
    /// which is required to generate the session.
    ///
    /// # Returns
    ///
    /// * A result containing the session details if successful, or an error
    /// if the session generation fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let session = kite_connect.generate_session(request_token);
    /// match session {
    ///     Ok(session) => println!("Session generated successfully!"),
    ///     Err(e) => println!("Error generating session: {}", e),
    /// }
    /// ```
    ///
    pub async fn generate_session(
        &mut self,
        request_token: &str,
    ) -> Result<KiteApiResponse<UserSession>> {
        // Yeah, this is quite a fetch.
        let api_key = self.client.http_config().credentials().api_key();
        let api_secret = self.client.http_config().credentials().api_secret();
        // Compute checksum needed for the API call
        let checksum = create_checksum(
            api_key.expose_secret().as_str(),
            request_token,
            api_secret.expose_secret().as_str(),
        );
        // Construct form parameters as per KiteConnect documentation
        //  ref: https://kite.trade/docs/connect/v3/user/#authentication-and-token-exchange
        let mut params: HashMap<&str, &str> = HashMap::new();
        params.insert("api_key", api_key.expose_secret().as_str());
        params.insert("request_token", request_token);
        params.insert("checksum", checksum.as_str());
        // info!("Params: {:?}", params);
        let kite_response: Result<KiteApiResponse<UserSession>> = self
            .client
            .post_form("/session/token", &params, &self.backoff)
            .await;
        match kite_response {
            Ok(kite_response) => {
                // Set the UserSession object on HTTPClient
                self.client.set_user_session(kite_response.data.clone());
                Ok(kite_response)
            }
            Err(err) => Err(err),
        }
    }

    /// Invalidates the `access_token` and destroys the current API session.
    ///
    /// After calling this method, the user will need to go through a new login
    /// flow to obtain a fresh `access_token` before any further interactions
    /// with the KiteConnect API can be made.
    ///
    /// This is useful for logging out a user or resetting their session for security reasons.
    ///
    pub async fn delete_session(&mut self) -> Result<KiteApiResponse<bool>> {
        match self
            .client
            .delete("/session/token", true, &self.backoff)
            .await
        {
            Ok(kite_response) => {
                // Remove the UserSession object from the HTTPClient
                self.client.set_user_session(None);
                Ok(kite_response)
            }
            Err(err) => Err(err),
        }
    }
}
