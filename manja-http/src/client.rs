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

use crate::api::{Alerts, BackoffPolicy, Charges, Gtt, Margins, Market, Orders, Session, User};
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
        let client = self.http_client();

        #[cfg(feature = "backoff")]
        {
            use backoff::future::retry;

            retry(backoff.clone(), || async {
                let request = request_baker().await.map_err(backoff::Error::Permanent)?;
                let method = request.method().to_string();
                let path = request.url().path().to_string();
                let span = tracing::info_span!("http.request", %method, %path);
                let _enter = span.enter();

                tracing::trace!("sending HTTP request");
                let response = client
                    .execute(request)
                    .await
                    .map_err(Error::from)
                    .map_err(backoff::Error::Permanent)?;
                let status = response.status();
                let json_response = response
                    .text()
                    .await
                    .map_err(Error::from)
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
                            .and_then(|error_type| Some(KiteApiException::from(error_type.as_str())))
                            .unwrap(),
                    };
                    tracing::error!(
                        status = status.as_u16(),
                        error_type = %kite_error.error_type.as_str(),
                        "Kite API error at {}",
                        path
                    );
                    if status.as_u16() == 429 {
                        tracing::warn!("Rate limited at endpoint: {}", path);
                        return Err(backoff::Error::transient(Error::from(kite_error)));
                    }
                    return Err(backoff::Error::Permanent(Error::from(kite_error)));
                }

                tracing::debug!(status = status.as_u16(), "HTTP request succeeded");
                Ok(json_response)
            })
            .await
        }

        #[cfg(not(feature = "backoff"))]
        {
            let request = request_baker().await?;
            let method = request.method().to_string();
            let path = request.url().path().to_string();
            let span = tracing::info_span!("http.request", %method, %path);
            let _enter = span.enter();

            tracing::trace!("sending HTTP request");
            let response = client.execute(request).await.map_err(Error::from)?;
            let status = response.status();
            let json_response = response.text().await.map_err(Error::from)?;
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
                    status = status.as_u16(),
                    error_type = %kite_error.error_type.as_str(),
                    "Kite API error at {}",
                    path
                );
                if status.as_u16() == 429 {
                    tracing::warn!("Rate limited at endpoint: {}", path);
                }
                return Err(Error::from(kite_error));
            }

            tracing::debug!(status = status.as_u16(), "HTTP request succeeded");
            Ok(json_response)
        }
    }
}
