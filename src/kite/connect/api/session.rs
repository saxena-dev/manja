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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mockito::ServerGuard;
    use tokio::join;

    use crate::kite::connect::client::test_utils::{
        add_mocks, get_manja_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
    };
    use crate::kite::connect::client::HTTPClient;
    use crate::kite::connect::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
    use crate::kite::connect::credentials::KiteCredentials;
    use crate::kite::connect::models::UserSession;
    use crate::kite::error::{KiteApiException, ManjaError};
    use crate::test_support::init_tracing;

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(
            ("POST", "/session/token"),
            "./kiteconnect-mocks/generate_session.json",
        );
        mmap
    }

    #[tokio::test]
    async fn test_generate_session_success() {
        init_tracing();
        let (server, mut manja_client) = get_manja_test_client().await;
        let server_ptr: *const ServerGuard = &server;
        log::debug!("Server @address: {:p}", server_ptr);
        let (_server,) = join!(add_mocks(server, mock_map()));

        let mut session_api = manja_client.session();
        let response = session_api
            .generate_session("dummy_request_token")
            .await
            .unwrap();
        let expected =
            read_to_object::<UserSession>("./kiteconnect-mocks/generate_session.json").unwrap();

        let session = response.data.expect("expected session data");
        assert_eq!(session.user_id, expected.user_id);
        assert!(manja_client.user_session().is_some());
    }

    #[test]
    fn test_delete_session_success_fixture_parses() {
        let root =
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        let path = std::path::Path::new(&root)
            .join("kiteconnect-mocks/session_logout.json");
        let json = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read fixture at {}: {}", path.display(), err));
        let response: KiteApiResponse<bool> = serde_json::from_str(&json).unwrap();
        assert_eq!(response.status, "success");
        assert_eq!(response.data, Some(true));
    }

    #[tokio::test]
    async fn test_generate_session_token_exception_error() {
        init_tracing();
        let mut server = mockito::Server::new_async().await;
        let credentials = KiteCredentials::new(
            "TEST_API_KEY",
            "TEST_API_SECRET",
            "TEST_USER_ID",
            "TEST_PASSWORD",
            "TEST_TOTP",
        );
        let config = Config::from_parts(
            server.url(),
            KITECONNECT_API_LOGIN.to_string(),
            KITECONNECT_API_REDIRECT.to_string(),
            credentials,
        );
        let mut client = HTTPClient::with_config(config);

        let _m = server
            .mock("POST", "/session/token")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Token is invalid or has expired",
                    "error_type": "TokenException"
                }"#,
            )
            .create_async()
            .await;

        let mut session_api = Session::new(&mut client);
        let err = session_api
            .generate_session("invalid_request_token")
            .await
            .expect_err("expected token exception error");

        match err {
            ManjaError::KiteApiError(api_err) => {
                assert_eq!(api_err.status_code, 403);
                assert!(matches!(
                    api_err.error_type,
                    KiteApiException::TokenException
                ));
                assert_eq!(api_err.error_type.as_str(), "TokenException");
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }
}
