//! Session API group: `/session/`
//!
//! The [Session] API group provides endpoints for managing user sessions,
//! including the two-step login flow required to authenticate with Kite Connect.

use std::collections::HashMap;

use secrecy::ExposeSecret;

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::Result;
use crate::utils::create_checksum;
use manja_core::models::{KiteApiResponse, UserSession};

/// User session related API endpoints for login and session management.
pub struct Session<'c> {
    /// A mutable reference to the HTTP client used for making API requests
    /// and storing a `UserSession` object after a successful login flow.
    pub client: &'c mut HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Session<'c> {
    /// Creates a new `Session` instance.
    pub fn new(client: &'c mut HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Session` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Generates a session using a `request token`.
    pub async fn generate_session(
        &mut self,
        request_token: &str,
    ) -> Result<KiteApiResponse<UserSession>> {
        // Generate checksum using the API secret and request token.
        let api_key = self.client.http_config().credentials().api_key();
        let api_secret = self.client.http_config().credentials().api_secret();
        let checksum =
            create_checksum(api_key.expose_secret(), api_secret.expose_secret(), request_token);

        let mut payload = HashMap::new();
        payload.insert("api_key", api_key.expose_secret().as_str());
        payload.insert("request_token", request_token);
        payload.insert("checksum", &checksum);

        let response = self
            .client
            .post_form::<UserSession, _>("/session/token", &payload, &self.backoff)
            .await?;

        if let Some(session) = response.data.clone() {
            self.client.set_user_session(Some(session));
        }

        Ok(response)
    }

    /// Invalidates the `access_token` and destroys the current API session.
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

