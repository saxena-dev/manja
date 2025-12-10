//! Asynchronous HTTP client.
//!
//! This module provides an asynchronous HTTP client for interacting with the
//! Kite Connect API. The `HTTPClient` struct is a thin facade over the
//! `manja-http` crate’s `HTTPClient`, preserving the original `manja` error
//! types and configuration surface while delegating actual HTTP behavior to
//! the shared transport crate.
//!
//! Existing callers should continue to use `crate::kite::connect::client::HTTPClient`
//! and `crate::kite::error::ManjaError` as before.

use core::future::Future;

use secrecy::{ExposeSecret, Secret};
use serde::{de::DeserializeOwned, Serialize};

use crate::kite::{
    connect::{
        api::{
            BackoffPolicy, Charges, Historical, Margins, Market, MutualFunds, Orders, Session,
            User,
        },
        config::Config,
        credentials::KiteCredentials,
        models::{
            HistoricalData, HistoricalInterval, KiteApiResponse, MfInstrument, UserSession,
        },
    },
    error::{map_deserialization_error, KiteApiException, ManjaError, Result},
    traits::KiteConfig,
};
use manja_core::error::KiteApiError;
use manja_core::traits::CoreApiEndpoints;
use manja_http::error::{Error as HttpError, Result as HttpResult};

/// An asynchronous Kite Connect client to make HTTP requests with.
///
/// This type preserves the existing `manja` facade while internally delegating
/// to `manja-http::HTTPClient` for all HTTP behavior.
#[derive(Clone)]
pub struct HTTPClient {
    inner: manja_http::HTTPClient,
    config: Config,
}

impl Default for HTTPClient {
    fn default() -> Self {
        let inner = manja_http::HTTPClient::default();
        let config = config_from_http_config(inner.http_config());
        Self { inner, config }
    }
}

impl HTTPClient {
    /// Create a HTTP client with explicit config.
    ///
    /// This preserves the existing `Config` type in `manja` by converting it
    /// into the transport crate’s configuration.
    pub fn with_config(config: Config) -> Self {
        let http_config = http_config_from_manja_config(&config);
        Self {
            inner: manja_http::HTTPClient::with_config(http_config),
            config,
        }
    }

    /// Exponential backoff for retrying rate limited requests.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.inner = self.inner.with_backoff(backoff);
        self
    }

    /// Add `UserSession` to the `HTTPClient`.
    pub fn with_user_session(mut self, user_session: UserSession) -> Self {
        self.inner = self.inner.with_user_session(user_session);
        self
    }

    /// Set `UserSession` on the `HTTPClient`.
    pub fn set_user_session(&mut self, user_session: Option<UserSession>) {
        self.inner.set_user_session(user_session);
    }

    /// User session, if it exists.
    pub fn user_session(&self) -> Option<&UserSession> {
        self.inner.user_session()
    }

    /// HTTP configuration and Kite user credentials.
    pub fn http_config(&self) -> &Config {
        &self.config
    }

    /// Reqwest HTTP Client.
    pub fn http_client(&self) -> &reqwest::Client {
        self.inner.http_client()
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
    pub fn historical(&mut self) -> Historical<'_> {
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
    pub fn gtt(&mut self) -> crate::kite::connect::api::Gtt<'_> {
        crate::kite::connect::api::Gtt::new(self)
    }

    /// To call [Alerts] related APIs using this client.
    pub fn alerts(&mut self) -> crate::kite::connect::api::Alerts<'_> {
        crate::kite::connect::api::Alerts::new(self)
    }

    /// To call Mutual Funds related APIs using this client.
    pub fn mutual_funds(&mut self) -> MutualFunds<'_> {
        MutualFunds::new(self)
    }

    // --- [ HTTP verb functions ] ---

    /// Make a GET request to {path} and return the response body.
    pub(crate) async fn get_raw(&self, path: &str, backoff: &BackoffPolicy) -> Result<String> {
        self.map_http_result(self.inner.get_raw(path, backoff).await, |s| s)
    }

    /// Make a GET request to {path} and deserialize the response body.
    pub(crate) async fn get<Model>(
        &self,
        path: &str,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        self.map_http_result(self.inner.get(path, backoff).await, |v| v)
    }

    /// Internal helper used by the facade Historical API group to call the
    /// transport-layer historical endpoint while preserving facade error types.
    pub(crate) async fn inner_historical_candles(
        &self,
        instrument_token: u32,
        interval: HistoricalInterval,
        from: chrono::NaiveDateTime,
        to: chrono::NaiveDateTime,
        continuous: bool,
        oi: bool,
    ) -> Result<KiteApiResponse<HistoricalData>> {
        self.map_http_result(
            self.inner
                .historical()
                .candles(instrument_token, interval, from, to, continuous, oi)
                .await,
            |v| v,
        )
    }

    /// Internal helper used by the facade Mutual Funds API group to call the
    /// transport-layer MF instruments endpoint while preserving facade error types.
    pub(crate) async fn inner_mutual_funds_instruments(&self) -> Result<Vec<MfInstrument>> {
        self.map_http_result(self.inner.mutual_funds().instruments().await, |v| v)
    }

    /// Make a GET request to {path} with given query and deserialize the response body.
    pub(crate) async fn get_with_query<Q, Model>(
        &self,
        path: &str,
        query: &Q,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Q: Serialize + ?Sized,
        Model: DeserializeOwned,
    {
        self.map_http_result(
            self.inner.get_with_query(path, query, backoff).await,
            |v| v,
        )
    }

    /// Make a POST request to {path} and deserialize the response body.
    pub(crate) async fn post<Model, Payload>(
        &self,
        path: &str,
        data: Payload,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        Payload: Serialize,
    {
        self.map_http_result(self.inner.post(path, data, backoff).await, |v| v)
    }

    /// POST a form at {path} and deserialize the response body into the generic
    /// `Model` type.
    pub(crate) async fn post_form<Model, F>(
        &self,
        path: &str,
        form: &F,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        F: Serialize + ?Sized,
    {
        self.map_http_result(
            self.inner.post_form(path, form, backoff).await,
            |v| v,
        )
    }

    /// Make a PUT request to {path} and deserialize the response body.
    pub(crate) async fn put<Model, Payload>(
        &self,
        path: &str,
        data: Payload,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        Payload: Serialize,
    {
        self.map_http_result(self.inner.put(path, data, backoff).await, |v| v)
    }

    /// Make a DELETE request to {path} and deserialize the response body.
    pub(crate) async fn delete<Model>(
        &self,
        path: &str,
        with_auth: bool,
        backoff: &BackoffPolicy,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        // We need to replicate the query parameter behavior using the public
        // config APIs and the session accessible via the facade.
        let request_baker = || async {
            let mut http_request_builder = self
                .http_client()
                .delete(self.http_config().url(path))
                .headers(self.http_config().headers(self.get_access_token()));

            if with_auth {
                let api_key = self.http_config().credentials().api_key();
                let query_vec = vec![
                    ("api_key", api_key.expose_secret().as_str()),
                    (
                        "access_token",
                        self.user_session()
                            .and_then(|session| {
                                Some(session.access_token.expose_secret().as_str())
                            })
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

    /// Execute a HTTP request asynchronously with backoff and return raw body.
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
                            ManjaError::Reqwest(err)
                        })
                        .map_err(backoff::Error::Permanent)?;
                    let status = response.status();
                    let json_response = response
                        .text()
                        .await
                        .map_err(|err| {
                            tracing::error!(error = %err, "HTTP body read error");
                            ManjaError::Reqwest(err)
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
                            return Err(backoff::Error::transient(ManjaError::KiteApiError(
                                kite_error,
                            )));
                        }
                        return Err(backoff::Error::Permanent(ManjaError::KiteApiError(
                            kite_error,
                        )));
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
                    ManjaError::Reqwest(err)
                })?;
            let status = response.status();
            let json_response = response
                .text()
                .await
                .map_err(|err| {
                    tracing::error!(error = %err, "HTTP body read error");
                    ManjaError::Reqwest(err)
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
                return Err(ManjaError::KiteApiError(kite_error));
            }

            tracing::debug!(
                status = status.as_u16(),
                attempt = 1_u32,
                "HTTP request succeeded"
            );
            Ok(json_response)
        }
    }

    fn get_access_token(&self) -> Option<Secret<String>> {
        self.inner
            .user_session()
            .map(|user_session| user_session.access_token.clone())
    }

    fn map_http_result<T, F>(&self, res: HttpResult<T>, map_ok: F) -> Result<T>
    where
        F: FnOnce(T) -> T,
    {
        res.map(map_ok).map_err(map_http_error)
    }
}

fn map_http_error(err: HttpError) -> ManjaError {
    match err {
        HttpError::KiteApi(e) => ManjaError::KiteApiError(e),
        HttpError::InvalidHeaderValue(e) => ManjaError::InvalidHeaderValueError(e),
        HttpError::Json(e) => ManjaError::JSONDeserialize(e),
        HttpError::Io(e) => ManjaError::IoError(e),
        HttpError::Reqwest(e) => ManjaError::Reqwest(e),
        HttpError::Internal(s) => ManjaError::Internal(s),
    }
}

fn config_from_http_config(transport_cfg: &manja_http::Config) -> Config {
    let creds = KiteCredentials::new(
        transport_cfg
            .credentials()
            .api_key()
            .expose_secret()
            .to_string(),
        transport_cfg
            .credentials()
            .api_secret()
            .expose_secret()
            .to_string(),
        transport_cfg
            .credentials()
            .user_id()
            .expose_secret()
            .to_string(),
        transport_cfg
            .credentials()
            .user_pwd()
            .expose_secret()
            .to_string(),
        transport_cfg
            .credentials()
            .totp_key()
            .expose_secret()
            .to_string(),
    );

    Config::from_parts(
        transport_cfg.api_base(),
        transport_cfg.api_login(),
        transport_cfg.api_redirect(),
        creds,
    )
}

fn http_config_from_manja_config(config: &Config) -> manja_http::Config {
    // Use the public `KiteConfig` trait implementation plus `KiteCredentials`
    // accessors to rebuild a transport config.
    let credentials = config.credentials();
    let http_creds = manja_http::KiteCredentials::new(
        credentials.api_key().expose_secret().to_string(),
        credentials.api_secret().expose_secret().to_string(),
        credentials.user_id().expose_secret().to_string(),
        credentials.user_pwd().expose_secret().to_string(),
        credentials.totp_key().expose_secret().to_string(),
    );

    manja_http::Config::from_parts(
        KiteConfig::api_base(config),
        KiteConfig::api_login(config),
        KiteConfig::api_redirect(config),
        http_creds,
    )
}
