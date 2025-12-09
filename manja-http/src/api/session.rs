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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::join;

    use crate::client::HTTPClient;
    use crate::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
    use crate::credentials::KiteCredentials;
    use crate::error::Error;
    use crate::test_utils::{
        add_mocks, get_http_test_client, read_to_object, resolve_fixture_path, APIEndpoint,
        HTTPMethod, TestResponse,
    };
    use manja_core::error::KiteApiException;
    use manja_core::models::{KiteApiResponse, UserSession};

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
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let mut session_api = client.session();
        let response = session_api
            .generate_session("dummy_request_token")
            .await
            .unwrap();
        let expected =
            read_to_object::<UserSession>("./kiteconnect-mocks/generate_session.json").unwrap();

        let session = response.data.expect("expected session data");
        assert_eq!(session.user_id, expected.user_id);
        assert!(client.user_session().is_some());
    }

    #[test]
    fn test_delete_session_success_fixture_parses() {
        let path = resolve_fixture_path("kiteconnect-mocks/session_logout.json");
        let json = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("failed to read fixture at {}: {}", path.display(), err)
        });
        let response: KiteApiResponse<bool> = serde_json::from_str(&json).unwrap();
        assert_eq!(response.status, "success");
        assert_eq!(response.data, Some(true));
    }

    #[tokio::test]
    async fn test_generate_session_token_exception_error() {
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
            Error::KiteApi(api_err) => {
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
