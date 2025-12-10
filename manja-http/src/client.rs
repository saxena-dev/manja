//! Asynchronous HTTP client for Kite Connect.
//!
//! This module provides an asynchronous HTTP client for interacting with the
//! Kite Connect HTTP API. The `HTTPClient` struct encapsulates a
//! `reqwest::Client` and includes methods for making HTTP requests to various
//! endpoints. The client supports optional retry mechanisms with exponential
//! backoff and handles user session management.

use core::future::Future;
use std::time::Duration;

use secrecy::{ExposeSecret, Secret};
use serde::{de::DeserializeOwned, Serialize};

use crate::api::{
    Alerts, BackoffPolicy, Charges, Gtt, Historical, Margins, Market, MutualFunds, Orders,
    Session, User,
};
use crate::config::Config;
use crate::error::{map_deserialization_error, Error, Result};
use manja_core::error::{KiteApiError, KiteApiException};
use manja_core::models::{KiteApiResponse, UserSession};

/// An asynchronous Kite Connect client to make HTTP requests with.
///
/// `HTTPClient` is a wrapper over `reqwest::Client` which holds a connection
/// pool internally. It is advisable to create one and **reuse** it.
#[derive(Clone)]
pub struct HTTPClient {
    client: reqwest::Client,
    config: Config,
    backoff: BackoffPolicy,
    session: Option<UserSession>,
}

impl Default for HTTPClient {
    fn default() -> Self {
        Self {
            // Default timeout for I/O operations: 10 seconds
            client: Self::default_reqwest_client(10),
            // Default config parameters are loaded from environment variables
            config: Config::default(),
            backoff: crate::api::create_backoff_policy(10),
            session: None,
        }
    }
}

impl HTTPClient {
    // Default `reqwest::Client` with timeout for I/O operations
    fn default_reqwest_client(timeout_seconds: u64) -> reqwest::Client {
        reqwest::ClientBuilder::new()
            .timeout(Duration::from_secs(timeout_seconds))
            .build()
            // This should not fail. Fallback to default `reqwest::Client`.
            .unwrap_or_else(|_| reqwest::Client::new())
    }

    fn get_access_token(&self) -> Option<Secret<String>> {
        match self.session {
            Some(ref user_session) => Some(user_session.access_token.clone()),
            None => None,
        }
    }

    /// Create a HTTP client with explicit config.
    pub fn with_config(config: Config) -> Self {
        Self {
            client: Self::default_reqwest_client(10),
            config,
            backoff: crate::api::create_backoff_policy(10),
            session: None,
        }
    }

    /// Exponential backoff for retrying rate-limited requests.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    /// Add `UserSession` to the `HTTPClient`.
    pub fn with_user_session(mut self, user_session: UserSession) -> Self {
        self.session = Some(user_session);
        self
    }

    /// Set `UserSession` on the `HTTPClient`.
    pub fn set_user_session(&mut self, user_session: Option<UserSession>) {
        self.session = user_session;
    }

    /// User session, if it exists.
    pub fn user_session(&self) -> Option<&UserSession> {
        self.session.as_ref()
    }

    /// HTTP configuration and Kite user credentials.
    pub fn http_config(&self) -> &Config {
        &self.config
    }

    /// Reqwest HTTP Client.
    pub fn http_client(&self) -> &reqwest::Client {
        &self.client
    }

    // --- [ API Groups ] ---

    /// To call [User] related APIs using this client.
    pub fn user(&self) -> User<'_> {
        User::new(self)
    }

    /// To call [Session] related APIs using this client.
    pub fn session(&mut self) -> Session<'_> {
        Session::new(self)
    }

    /// To call [Orders] related APIs using this client.
    pub fn orders(&mut self) -> Orders<'_> {
        Orders::new(self)
    }

    /// To call [Market] related APIs using this client.
    pub fn market(&mut self) -> Market<'_> {
        Market::new(self)
    }

    /// To call [Historical] related APIs using this client.
    pub fn historical(&self) -> Historical<'_> {
        Historical::new(self)
    }

    /// To call [Margins] related APIs using this client.
    pub fn margins(&mut self) -> Margins<'_> {
        Margins::new(self)
    }

    /// To call [Charges] related APIs using this client.
    pub fn charges(&mut self) -> Charges<'_> {
        Charges::new(self)
    }

    /// To call [Gtt] related APIs using this client.
    pub fn gtt(&mut self) -> Gtt<'_> {
        Gtt::new(self)
    }

    /// To call [Alerts] related APIs using this client.
    pub fn alerts(&mut self) -> Alerts<'_> {
        Alerts::new(self)
    }

    /// To call Mutual Funds related APIs using this client.
    pub fn mutual_funds(&self) -> MutualFunds<'_> {
        MutualFunds::new(self)
    }

    // --- [ HTTP verb functions ] ---

    /// Make a GET request to {path} and return the response body.
    pub async fn get_raw(&self, path: &str, backoff: &BackoffPolicy) -> Result<String> {
        let request_baker = || async {
            Ok(self
                .client
                .get(self.config.url(path))
                // Fetch access token for protected endpoints, if available
                .headers(self.config.headers(self.get_access_token()))
                .build()?)
        };

        self.execute_raw(backoff, request_baker).await
    }

    /// Make a GET request to {path} and deserialize the response body.
    pub async fn get<Model>(
        &self,
        path: &str,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        let request_baker = || async {
            Ok(self
                .client
                .get(self.config.url(path))
                .headers(self.config.headers(self.get_access_token()))
                .build()?)
        };

        self.execute(backoff, request_baker).await
    }

    /// Make a GET request to {path} with given query and deserialize the response body.
    pub async fn get_with_query<Q, Model>(
        &self,
        path: &str,
        query: &Q,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Q: Serialize + ?Sized,
        Model: DeserializeOwned,
    {
        let request_baker = || async {
            Ok(self
                .client
                .get(self.config.url(path))
                .query(query)
                .headers(self.config.headers(self.get_access_token()))
                .build()?)
        };

        self.execute(backoff, request_baker).await
    }

    /// Make a POST request to {path} and deserialize the response body.
    pub async fn post<Model, Payload>(
        &self,
        path: &str,
        data: Payload,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        Payload: Serialize,
    {
        let request_baker = || async {
            Ok(self
                .client
                .post(self.config.url(path))
                .headers(self.config.headers(self.get_access_token()))
                .json(&data)
                .build()?)
        };

        self.execute(backoff, request_baker).await
    }

    /// POST a form at {path} and deserialize the response body into the generic
    /// `Model` type.
    pub async fn post_form<Model, F>(
        &self,
        path: &str,
        form: &F,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        F: Serialize + ?Sized,
    {
        let request_baker = || async {
            Ok(self
                .client
                .post(self.config.url(path))
                .headers(self.config.headers(self.get_access_token()))
                .form(form)
                .build()?)
        };

        self.execute(backoff, request_baker).await
    }

    /// Make a PUT request to {path} and deserialize the response body.
    pub async fn put<Model, Payload>(
        &self,
        path: &str,
        data: Payload,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        Payload: Serialize,
    {
        let request_baker = || async {
            Ok(self
                .client
                .put(self.config.url(path))
                .headers(self.config.headers(self.get_access_token()))
                .json(&data)
                .build()?)
        };

        self.execute(backoff, request_baker).await
    }

    /// Make a DELETE request to {path} and deserialize the response body.
    pub async fn delete<Model>(
        &self,
        path: &str,
        with_auth: bool,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        let request_baker = || async {
            let mut http_request_builder = self
                .client
                .delete(self.config.url(path))
                .headers(self.config.headers(self.get_access_token()));

            if with_auth {
                let api_key = self.http_config().credentials().api_key();
                // Construct Vec<&str, &str> for query construction
                let query_vec = vec![
                    ("api_key", api_key.expose_secret().as_str()),
                    (
                        "access_token",
                        self.user_session()
                            .and_then(|session| Some(session.access_token.expose_secret().as_str()))
                            .unwrap_or_else(|| &"(ﾉﾟ0ﾟ)ﾉ~"),
                    ),
                ];
                http_request_builder = http_request_builder.query(&query_vec);
            }
            Ok(http_request_builder.build()?)
        };

        self.execute(backoff, request_baker).await
    }

    /// Execute a HTTP request asynchronously with backoff and deserialize JSON.
    async fn execute<Model, RB, Fut>(
        &self,
        backoff: &BackoffPolicy,
        request_baker: RB,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        RB: Fn() -> Fut,
        Fut: Future<Output = Result<reqwest::Request>>,
    {
        let json_response = self.execute_raw::<RB, Fut>(backoff, request_baker).await?;

        let model: KiteApiResponse<Model> = serde_json::from_str(&json_response)
            .map_err(|e| map_deserialization_error(e, &json_response))?;

        Ok(model)
    }

    /// Execute a HTTP request asynchronously with backoff and return the raw body.
    async fn execute_raw<RB, Fut>(
        &self,
        backoff: &BackoffPolicy,
        request_baker: RB,
    ) -> Result<String>
    where
        RB: Fn() -> Fut,
        Fut: Future<Output = Result<reqwest::Request>>,
    {
        #[cfg(feature = "backoff")]
        {
            use backoff::future::retry;

            let client = self.http_client();
            let mut attempt: u32 = 0;

            // NOTE: The retry and rate-limit semantics exercised here are
            // validated by time-controlled tests in this crate (see
            // `tests::rate_limited_requests_are_retried_with_backoff` and
            // related cases), which lock in the expected 429 vs non-429
            // behavior.
            retry(backoff.clone(), || {
                attempt += 1;
                let attempt_no = attempt;
                let request_fut = request_baker();

                async move {
                    let request =
                        request_fut.await.map_err(backoff::Error::Permanent)?;
                    let method = request.method().to_string();
                    let path = request.url().path().to_string();
                    let span = tracing::info_span!(
                        "http.request",
                        %method,
                        %path,
                        attempt = attempt_no
                    );
                    let _enter = span.enter();

                    tracing::trace!("sending HTTP request");
                    let response = client
                        .execute(request)
                        .await
                        .map_err(|err| {
                            tracing::error!(error = %err, "HTTP transport error");
                            Error::from(err)
                        })
                        .map_err(backoff::Error::Permanent)?;
                    let status = response.status();
                    let json_response = response
                        .text()
                        .await
                        .map_err(|err| {
                            tracing::error!(error = %err, "HTTP body read error");
                            Error::from(err)
                        })
                        .map_err(backoff::Error::Permanent)?;
                    if !status.is_success() {
                        let kite_response: KiteApiResponse<Option<String>> =
                            serde_json::from_str(&json_response)
                                .map_err(|e| map_deserialization_error(e, &json_response))
                                .map_err(backoff::Error::Permanent)?;
                        let kite_error = KiteApiError {
                            endpoint: path.clone(),
                            status_code: status.as_u16(),
                            message: kite_response.message,
                            error_type: kite_response
                                .error_type
                                .and_then(|error_type| {
                                    Some(KiteApiException::from(error_type.as_str()))
                                })
                                .unwrap(),
                        };
                        tracing::error!(
                            status = status.as_u16(),
                            error_type = %kite_error.error_type.as_str(),
                            attempt = attempt_no,
                            "Kite API error at {}",
                            path
                        );
                        if status.as_u16() == 429 {
                            tracing::warn!(
                                status = status.as_u16(),
                                attempt = attempt_no,
                                "Rate limited at endpoint: {} (retrying with backoff)",
                                path
                            );
                            return Err(backoff::Error::transient(Error::from(kite_error)));
                        }
                        return Err(backoff::Error::Permanent(Error::from(kite_error)));
                    }

                    tracing::debug!(
                        status = status.as_u16(),
                        attempt = attempt_no,
                        "HTTP request succeeded"
                    );
                    Ok(json_response)
                }
            })
            .await
        }

        #[cfg(not(feature = "backoff"))]
        {
            let client = self.http_client();
            let request = request_baker().await?;
            let method = request.method().to_string();
            let path = request.url().path().to_string();
            let span = tracing::info_span!("http.request", %method, %path);
            let _enter = span.enter();

            tracing::trace!("sending HTTP request");
            let response = client
                .execute(request)
                .await
                .map_err(|err| {
                    tracing::error!(error = %err, "HTTP transport error");
                    Error::from(err)
                })?;
            let status = response.status();
            let json_response = response.text().await.map_err(|err| {
                tracing::error!(error = %err, "HTTP body read error");
                Error::from(err)
            })?;
            if !status.is_success() {
                let kite_response: KiteApiResponse<Option<String>> =
                    serde_json::from_str(&json_response)
                        .map_err(|e| map_deserialization_error(e, &json_response))?;
                let kite_error = KiteApiError {
                    endpoint: path.clone(),
                    status_code: status.as_u16(),
                    message: kite_response.message,
                    error_type: kite_response
                        .error_type
                        .and_then(|error_type| Some(KiteApiException::from(error_type.as_str())))
                        .unwrap(),
                };
                tracing::error!(
                    attempt = 1_u32,
                    status = status.as_u16(),
                    error_type = %kite_error.error_type.as_str(),
                    "Kite API error at {}",
                    path
                );
                if status.as_u16() == 429 {
                    tracing::warn!(
                        status = status.as_u16(),
                        attempt = 1_u32,
                        "Rate limited at endpoint: {}",
                        path
                    );
                }
                return Err(Error::from(kite_error));
            }

            tracing::debug!(
                status = status.as_u16(),
                attempt = 1_u32,
                "HTTP request succeeded"
            );
            Ok(json_response)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
    use crate::credentials::KiteCredentials;
    use crate::error::Error;
    use manja_core::error::KiteApiException;
    use std::time::Duration;

    /// When backoff is enabled, a 429 rate-limit response must be treated as
    /// transient: the client retries according to the backoff policy until a
    /// later success response is returned.
    #[cfg(feature = "backoff")]
    #[tokio::test(start_paused = true)]
    async fn rate_limited_requests_are_retried_with_backoff() {
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
        let client = HTTPClient {
            client: reqwest::Client::new(),
            config,
            backoff: crate::api::create_backoff_policy(10),
            session: None,
        };

        let path = "/backoff/test/retry";

        // First two attempts: 429 "Too Many Requests" with a valid error body.
        // These should be surfaced to `backoff` as transient errors and retried.
        let _rate_limit_mock = server
            .mock("GET", path)
            .with_status(429)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Rate limit exceeded",
                    "error_type": "GeneralException"
                }"#,
            )
            .expect(2)
            .create_async()
            .await;

        // Third attempt: succeed with a simple JSON payload. The overall call
        // should resolve successfully once this attempt is reached.
        let _success_mock = server
            .mock("GET", path)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "success",
                    "data": "ok",
                    "message": null,
                    "error_type": null
                }"#,
            )
            .expect(1)
            .create_async()
            .await;

        // Use a small, known interval so advancing virtual time by a few
        // seconds is enough to trigger multiple retries.
        let backoff = crate::api::create_backoff_policy(1);

        let client_clone = client.clone();
        let backoff_clone = backoff.clone();
        let path_owned = path.to_string();

        let handle = tokio::spawn(async move {
            client_clone
                .get::<String>(&path_owned, &backoff_clone)
                .await
        });

        // Drive virtual time forward so that `backoff::future::retry` can
        // progress through multiple attempts without real sleeps. Because
        // Tokio time is paused, the request would otherwise never advance.
        for _ in 0..5 {
            tokio::time::advance(Duration::from_secs(1)).await;
        }

        let response = handle
            .await
            .expect("HTTP client task join error")
            .expect("expected successful response after retries");

        // We expect that:
        // - At least one retry was attempted after a 429.
        // - The final outcome reflects the 200 response.
        assert_eq!(response.status, "success");
        assert_eq!(response.data.as_deref(), Some("ok"));
    }

    /// When `max_elapsed_time` is set on the backoff policy, an endless stream
    /// of 429 responses must cause the overall operation to fail once the
    /// maximum elapsed time budget is exhausted.
    #[cfg(feature = "backoff")]
    #[tokio::test(start_paused = true)]
    async fn retries_stop_after_max_elapsed_time() {
        use backoff::ExponentialBackoffBuilder;

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
        let client = HTTPClient {
            client: reqwest::Client::new(),
            config,
            backoff: crate::api::create_backoff_policy(10),
            session: None,
        };

        let path = "/backoff/test/max_elapsed";

        // Always respond with 429 so the retry loop never encounters success.
        let _rate_limit_mock = server
            .mock("GET", path)
            .with_status(429)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Rate limit exceeded",
                    "error_type": "GeneralException"
                }"#,
            )
            .create_async()
            .await;

        // Configure a bounded backoff policy: short interval and a small
        // `max_elapsed_time` so the retry loop gives up deterministically.
        let backoff = ExponentialBackoffBuilder::new()
            .with_initial_interval(Duration::from_secs(1))
            .with_multiplier(1.0)
            .with_max_interval(Duration::from_secs(1))
            .with_max_elapsed_time(Some(Duration::from_secs(3)))
            .build();

        let client_clone = client.clone();
        let backoff_clone = backoff.clone();
        let path_owned = path.to_string();

        let handle = tokio::spawn(async move {
            client_clone
                .get::<String>(&path_owned, &backoff_clone)
                .await
        });

        // Advance virtual time far beyond the `max_elapsed_time` budget so the
        // backoff implementation has a chance to stop retrying and return.
        for _ in 0..10 {
            tokio::time::advance(Duration::from_secs(1)).await;
        }

        let result = handle.await.expect("HTTP client task join error");
        // We only care that the operation failed (no infinite retries). The
        // exact error shape is delegated to the `backoff` crate and our
        // error-mapping logic.
        assert!(result.is_err(), "expected request to fail after exceeding max elapsed time");
    }

    /// When backoff is enabled, non-429 error responses (e.g., 500) must be
    /// treated as permanent errors and not retried.
    #[cfg(feature = "backoff")]
    #[tokio::test(start_paused = true)]
    async fn non_429_errors_are_not_retried_with_backoff() {
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
        let client = HTTPClient {
            client: reqwest::Client::new(),
            config,
            backoff: crate::api::create_backoff_policy(10),
            session: None,
        };

        let path = "/backoff/test/non_429";

        // Single 500 response with a valid Kite-style error body; this should
        // be mapped to a permanent `Error::KiteApi` without any retry.
        let _error_mock = server
            .mock("GET", path)
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Internal server error",
                    "error_type": "GeneralException"
                }"#,
            )
            .expect(1)
            .create_async()
            .await;

        // Backoff is still configured, but since the error is non-429 it
        // should not be used for retries at all.
        let backoff = crate::api::create_backoff_policy(1);

        let err = client
            .get::<String>(path, &backoff)
            .await
            .expect_err("expected non-429 error to be treated as permanent");

        // Assert that the error was classified as a permanent Kite API error
        // with the expected status code and error type.
        match err {
            Error::KiteApi(api_err) => {
                assert_eq!(api_err.status_code, 500);
                assert!(matches!(
                    api_err.error_type,
                    KiteApiException::GeneralException
                ));
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }

    /// When the `backoff` feature is disabled, 429 responses must not be
    /// retried at all: the client performs a single attempt and returns the
    /// rate-limit error immediately.
    #[cfg(not(feature = "backoff"))]
    #[tokio::test]
    async fn rate_limited_responses_are_not_retried_without_backoff() {
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
        let client = HTTPClient::with_config(config);

        let path = "/backoff/test/without_feature";

        let _rate_limit_mock = server
            .mock("GET", path)
            .with_status(429)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Rate limit exceeded",
                    "error_type": "GeneralException"
                }"#,
            )
            .expect(1)
            .create_async()
            .await;

        let backoff = crate::api::create_backoff_policy(1);

        let err = client
            .get::<String>(path, &backoff)
            .await
            .expect_err("expected 429 to be returned without retry when backoff is disabled");

        match err {
            Error::KiteApi(api_err) => {
                assert_eq!(api_err.status_code, 429);
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }
}
