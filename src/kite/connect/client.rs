//! Asynchronous HTTP client.
//!
//! [`HTTPClient`] owns a pooled HTTP transport, its [`Config`], and an
//! optional immutable [`Credentials`] snapshot, and hands out the resource
//! facades (`user()`, `orders()`, …). Cloning a client shares its transport;
//! [`HTTPClient::with_credentials`] builds a client for different
//! credentials on the same transport without changing the original.
//!
//! Every response is classified totally and without panicking:
//!
//! - success requires a 2xx status **and** a JSON envelope with
//!   `status = "success"` and a `data` payload that matches the endpoint's
//!   type (`kite-api-docs/docs/connect/v3/response-structure.md:15`);
//! - a non-2xx status or `status = "error"` is never `Ok`, whether or not
//!   `error_type` is present or known (`response-structure.md:28`);
//! - HTML, malformed or truncated JSON, a missing payload and a body larger
//!   than its bound are errors that keep the HTTP status.
//!
//! Failures are [`HttpError`]s with stage evidence; see
//! [`crate::kite::error`]. Client construction failures are returned, never
//! replaced by a differently configured transport.
//!
use core::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use backoff::ExponentialBackoff;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use crate::kite::{
    connect::{
        admission::{Admission, RateClass},
        api::{Charges, Margins, Market, Orders, Session, User},
        config::Config,
        credentials::{AccessToken, ApiKey, Credentials},
        models::{KiteApiResponse, UserSession},
    },
    error::{BrokerError, HttpError, HttpErrorKind, KiteApiException, ManjaError, Result},
    obs::schema::{Endpoint, Method},
    protocol::Inbound,
    traits::KiteConfig,
};

/// Attempt timeout applied by the underlying transport.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

struct Transport {
    http: reqwest::Client,
    config: Config,
    admission: Admission,
    in_flight: Arc<tokio::sync::Semaphore>,
}

/// An asynchronous Kite Connect HTTP client.
///
/// Create one and reuse it: it holds a connection pool, and clones share it.
/// A client's credentials are an immutable snapshot; to use other
/// credentials, derive a new client with [`Self::with_credentials`].
#[derive(Clone)]
pub struct HTTPClient {
    transport: Arc<Transport>,
    credentials: Option<Credentials>,
    backoff: ExponentialBackoff,
}

impl std::fmt::Debug for HTTPClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HTTPClient")
            .field("config", &self.transport.config)
            .field("credentials", &self.credentials)
            .finish()
    }
}

/// Builder for [`HTTPClient`].
///
/// A client built without [`Self::admission`] gets its own budget scope.
/// Give independently built clients one [`Admission`] to make them share
/// quota windows; clones and [`HTTPClient::with_credentials`] share it
/// automatically.
#[must_use]
pub struct HttpClientBuilder {
    config: Config,
    admission: Option<Admission>,
    credentials: Option<Credentials>,
}

impl HttpClientBuilder {
    /// Draw quota from `admission`, shared with other clients.
    pub fn admission(mut self, admission: Admission) -> Self {
        self.admission = Some(admission);
        self
    }

    /// Authenticate with `credentials`.
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Build the client. Fails with a configuration error if the HTTP
    /// transport cannot be built.
    pub fn build(self) -> Result<HTTPClient> {
        let mut client =
            HTTPClient::build_transport(self.config, self.admission.unwrap_or_default())?;
        client.credentials = self.credentials;
        Ok(client)
    }
}

impl HTTPClient {
    /// A builder for a client using `config`.
    pub fn builder(config: Config) -> HttpClientBuilder {
        HttpClientBuilder {
            config,
            admission: None,
            credentials: None,
        }
    }

    /// A client for `config` with no credentials and its own budget scope.
    ///
    /// Fails with a configuration error if the HTTP transport cannot be
    /// built.
    pub fn new(config: Config) -> Result<Self> {
        Self::builder(config).build()
    }

    fn build_transport(config: Config, admission: Admission) -> Result<Self> {
        let http = reqwest::ClientBuilder::new()
            .timeout(ATTEMPT_TIMEOUT)
            .build()
            .map_err(|e| {
                HttpError::new(
                    HttpErrorKind::Configuration,
                    Method::Get,
                    Endpoint::Unknown,
                    crate::kite::error::TransportStage::NotStarted,
                )
                .with_detail("the HTTP transport could not be built")
                .with_source(e.without_url())
            })?;
        let in_flight = Arc::new(tokio::sync::Semaphore::new(config.limits().in_flight()));
        Ok(Self {
            transport: Arc::new(Transport {
                http,
                config,
                admission,
                in_flight,
            }),
            credentials: None,
            backoff: Default::default(),
        })
    }

    /// The admission scope this client draws quota from.
    pub fn admission(&self) -> &Admission {
        &self.transport.admission
    }

    /// Same as [`Self::new`].
    pub fn with_config(config: Config) -> Result<Self> {
        Self::new(config)
    }

    /// Exponential backoff for retrying [rate limited](https://kite.trade/docs/connect/v3/exceptions/#api-rate-limit) requests.
    ///
    pub fn with_backoff(mut self, backoff: backoff::ExponentialBackoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// A client sharing this client's transport, authenticated with
    /// `credentials`. The original client is unchanged.
    pub fn with_credentials(&self, credentials: Credentials) -> Self {
        Self {
            credentials: Some(credentials),
            ..self.clone()
        }
    }

    /// A client authenticated with the API key and access token of a token
    /// exchange response. Only those two values are kept.
    pub fn with_user_session(self, user_session: UserSession) -> Result<Self> {
        let credentials = credentials_from_session(&user_session)?;
        Ok(self.with_credentials(credentials))
    }

    /// The credential snapshot, if any.
    pub fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    // Used only by the legacy `Session` facade until it is replaced.
    pub(crate) fn replace_credentials(&mut self, credentials: Option<Credentials>) {
        self.credentials = credentials;
    }

    /// HTTP configurations and Kite user credentials.
    ///
    pub fn http_config(&self) -> &Config {
        &self.transport.config
    }

    // --- [ API Groups ] ---

    /// To call [User] related APIs using this client.
    ///
    pub fn user(&self) -> User<'_> {
        User::new(self)
    }

    /// To call [Session] related APIs using this client.
    ///
    pub fn session(&mut self) -> Session<'_> {
        Session::new(self)
    }

    /// To call [Orders] related APIs using this client.
    ///
    pub fn orders(&mut self) -> Orders<'_> {
        Orders::new(self)
    }

    /// To call [Market] related APIs using this client.
    ///
    pub fn market(&mut self) -> Market<'_> {
        Market::new(self)
    }

    /// To call [Margins] related APIs using this client.
    ///
    pub fn margins(&mut self) -> Margins<'_> {
        Margins::new(self)
    }

    /// To call [Charges] related APIs using this client.
    ///
    pub fn charges(&mut self) -> Charges<'_> {
        Charges::new(self)
    }

    // --- [ HTTP verb functions ] ---

    /// GET `path` and return the raw (CSV) body.
    pub(crate) async fn get_raw(&self, path: &str, backoff: &ExponentialBackoff) -> Result<String> {
        let (status, body, attempt) = self
            .execute(reqwest::Method::GET, path, backoff, BodyKind::Csv, |rb| rb)
            .await?;
        let (method, endpoint) = labels(&reqwest::Method::GET, path);
        classify_text(status, &body, method, endpoint).map_err(|e| e.with_attempt(attempt).into())
    }

    /// GET `path` and decode the response envelope.
    pub(crate) async fn get<Model>(
        &self,
        path: &str,
        backoff: &ExponentialBackoff,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        self.json(reqwest::Method::GET, path, backoff, |rb| rb)
            .await
    }

    /// GET `path` with a query and decode the response envelope.
    pub(crate) async fn get_with_query<Q, Model>(
        &self,
        path: &str,
        query: &Q,
        backoff: &ExponentialBackoff,
    ) -> Result<KiteApiResponse<Model>>
    where
        Q: Serialize + ?Sized,
        Model: DeserializeOwned,
    {
        self.json(reqwest::Method::GET, path, backoff, |rb| rb.query(query))
            .await
    }

    /// POST a JSON body to `path` and decode the response envelope.
    pub(crate) async fn post<Model, Payload>(
        &self,
        path: &str,
        data: Payload,
        backoff: &ExponentialBackoff,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        Payload: Serialize,
    {
        self.json(reqwest::Method::POST, path, backoff, |rb| rb.json(&data))
            .await
    }

    /// POST a form to `path` and decode the response envelope.
    pub(crate) async fn post_form<Model, F>(
        &self,
        path: &str,
        form: &F,
        backoff: &ExponentialBackoff,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        F: Serialize + ?Sized,
    {
        self.json(reqwest::Method::POST, path, backoff, |rb| rb.form(form))
            .await
    }

    /// PUT a JSON body to `path` and decode the response envelope.
    pub(crate) async fn put<Model, Payload>(
        &self,
        path: &str,
        data: Payload,
        backoff: &ExponentialBackoff,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        Payload: Serialize,
    {
        self.json(reqwest::Method::PUT, path, backoff, |rb| rb.json(&data))
            .await
    }

    /// DELETE `path` and decode the response envelope. With `with_auth`, the
    /// API key and access token are also sent as query parameters, as the
    /// session-logout endpoint documents
    /// (`kite-api-docs/docs/connect/v3/user.md:321-323`).
    pub(crate) async fn delete<Model>(
        &self,
        path: &str,
        with_auth: bool,
        backoff: &ExponentialBackoff,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        let query: Vec<(&str, &str)> = match (&self.credentials, with_auth) {
            (Some(c), true) => vec![
                ("api_key", c.api_key().as_str()),
                ("access_token", c.access_token().expose_secret()),
            ],
            _ => Vec::new(),
        };
        self.json(reqwest::Method::DELETE, path, backoff, |rb| {
            if query.is_empty() {
                rb
            } else {
                rb.query(&query)
            }
        })
        .await
    }

    async fn json<Model, B>(
        &self,
        method: reqwest::Method,
        path: &str,
        backoff: &ExponentialBackoff,
        build: B,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    {
        let (status, body, attempt) = self
            .execute(method.clone(), path, backoff, BodyKind::Json, build)
            .await?;
        let (m, endpoint) = labels(&method, path);
        classify_json(status, &body, m, endpoint).map_err(|e| e.with_attempt(attempt).into())
    }

    /// Run attempts under the legacy backoff policy, which retries only
    /// HTTP 429, and return the final status, body and attempt number.
    async fn execute<B>(
        &self,
        method: reqwest::Method,
        path: &str,
        backoff: &ExponentialBackoff,
        kind: BodyKind,
        build: B,
    ) -> Result<(u16, Vec<u8>, u32)>
    where
        B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    {
        let attempts = AtomicU32::new(0);
        let (m, endpoint) = labels(&method, path);
        backoff::future::retry(backoff.clone(), || async {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed) + 1;
            let (status, body) = self
                .attempt(&method, path, kind, &build, m, endpoint, attempt)
                .await
                .map_err(|e| backoff::Error::Permanent(ManjaError::from(e)))?;
            if status == 429 {
                tracing::warn!(endpoint = endpoint.as_str(), "rate limited");
                let err = classify_json::<Value>(status, &body, m, endpoint)
                    .err()
                    .unwrap_or_else(|| {
                        HttpError::new(
                            HttpErrorKind::HttpStatus,
                            m,
                            endpoint,
                            crate::kite::error::TransportStage::ResponseReceived,
                        )
                        .with_status(429)
                    })
                    .with_attempt(attempt);
                return Err(backoff::Error::transient(ManjaError::from(err)));
            }
            Ok((status, body, attempt))
        })
        .await
    }

    /// One transport attempt: build, send and read a bounded body.
    // Internal error path; the error is boxed into `ManjaError::Http`.
    #[allow(clippy::too_many_arguments, clippy::result_large_err)]
    async fn attempt<B>(
        &self,
        method: &reqwest::Method,
        path: &str,
        kind: BodyKind,
        build: &B,
        m: Method,
        endpoint: Endpoint,
        attempt: u32,
    ) -> std::result::Result<(u16, Vec<u8>), HttpError>
    where
        B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    {
        use crate::kite::error::TransportStage as Stage;
        let not_started = |detail: &str| {
            HttpError::new(HttpErrorKind::Configuration, m, endpoint, Stage::NotStarted)
                .with_attempt(attempt)
                .with_detail(detail)
        };
        let admission = &self.transport.admission;
        let wait = admission.limits().wait();
        let (class, order_id) = rate_key(m, endpoint, path);
        let admitted = |e: &dyn std::fmt::Display| {
            HttpError::new(HttpErrorKind::Admission, m, endpoint, Stage::NotStarted)
                .with_attempt(attempt)
                .with_detail(&e.to_string())
        };
        let grant = admission
            .acquire(class, order_id.as_deref(), wait)
            .await
            .map_err(|e| admitted(&e))?;
        let _in_flight =
            tokio::time::timeout(wait, self.transport.in_flight.clone().acquire_owned())
                .await
                .map_err(|_| admitted(&"no in-flight attempt slot within the admission wait"))?
                .map_err(|_| admitted(&"the transport is closed"))?;
        let url = self.transport.config.url(path);
        let mut rb = self
            .transport
            .http
            .request(method.clone(), url)
            .header("X-Kite-Version", "3");
        if let Some(creds) = &self.credentials {
            let mut value = HeaderValue::from_str(creds.authorization_header().expose())
                .map_err(|_| not_started("the Authorization header could not be built"))?;
            value.set_sensitive(true);
            rb = rb.header(AUTHORIZATION, value);
        }
        let request = build(rb).build().map_err(|e| {
            not_started("the request could not be built").with_source(e.without_url())
        })?;
        // Dispatch now: the admitted capacity is used from here on.
        grant.consume();
        let mut response = self.transport.http.execute(request).await.map_err(|e| {
            // A connect failure is affirmative evidence that no request
            // bytes left the process; anything later is not.
            let stage = if e.is_connect() {
                Stage::NotStarted
            } else {
                Stage::Started
            };
            let err =
                HttpError::new(HttpErrorKind::Transport, m, endpoint, stage).with_attempt(attempt);
            let err = if e.is_timeout() {
                err.with_timeout()
            } else {
                err
            };
            err.with_source(e.without_url())
        })?;
        let status = response.status().as_u16();
        let limit = match kind {
            BodyKind::Json => self.transport.config.limits().json_body_bytes(),
            BodyKind::Csv => self.transport.config.limits().csv_body_bytes(),
        };
        let received = |kind: HttpErrorKind| {
            HttpError::new(kind, m, endpoint, Stage::ResponseReceived)
                .with_status(status)
                .with_attempt(attempt)
        };
        if response.content_length().is_some_and(|n| n > limit as u64) {
            return Err(
                received(HttpErrorKind::Decode).with_detail("response body exceeds its bound")
            );
        }
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len() + chunk.len() > limit {
                        return Err(received(HttpErrorKind::Decode)
                            .with_detail("response body exceeds its bound"));
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    let err = received(HttpErrorKind::Transport)
                        .with_detail("the response body was cut off");
                    let err = if e.is_timeout() {
                        err.with_timeout()
                    } else {
                        err
                    };
                    return Err(err.with_source(e.without_url()));
                }
            }
        }
        Ok((status, body))
    }
}

#[derive(Clone, Copy)]
enum BodyKind {
    Json,
    Csv,
}

pub(crate) fn credentials_from_session(session: &UserSession) -> Result<Credentials> {
    use secrecy::ExposeSecret;
    Ok(Credentials::from_parts(
        ApiKey::new(session.api_key.expose_secret().as_str())?,
        AccessToken::new(session.access_token.expose_secret().as_str())?,
    ))
}

/// The rate class of a request and, for a modification, its order ID.
fn rate_key(method: Method, endpoint: Endpoint, path: &str) -> (RateClass, Option<String>) {
    let class = RateClass::of(method, endpoint);
    let order_id = match class {
        RateClass::OrderModification => path
            .split('?')
            .next()
            .and_then(|p| p.trim_matches('/').rsplit('/').next())
            .map(str::to_string),
        _ => None,
    };
    (class, order_id)
}

fn labels(method: &reqwest::Method, path: &str) -> (Method, Endpoint) {
    let m = match *method {
        reqwest::Method::POST => Method::Post,
        reqwest::Method::PUT => Method::Put,
        reqwest::Method::DELETE => Method::Delete,
        _ => Method::Get,
    };
    (m, endpoint_template(m, path))
}

/// Map a concrete request path to its endpoint template, so no dynamic
/// segment (variety, order ID, exchange) ever reaches metadata or labels.
pub(crate) fn endpoint_template(method: Method, path: &str) -> Endpoint {
    let path = path.split('?').next().unwrap_or(path);
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match segments.as_slice() {
        ["user", "profile"] => Endpoint::UserProfile,
        ["user", "margins"] => Endpoint::UserMargins,
        ["user", "margins", _] => Endpoint::UserMarginsSegment,
        ["orders"] => Endpoint::Orders,
        ["orders", _] if method == Method::Post => Endpoint::OrdersVariety,
        ["orders", _] => Endpoint::OrdersId,
        ["orders", _, "trades"] if method == Method::Get => Endpoint::OrdersIdTrades,
        ["orders", _, _] => Endpoint::OrdersVarietyId,
        ["trades"] => Endpoint::Trades,
        ["portfolio", "holdings"] => Endpoint::Holdings,
        ["portfolio", "holdings", "auctions"] => Endpoint::HoldingsAuctions,
        ["portfolio", "positions"] => Endpoint::Positions,
        ["instruments"] => Endpoint::Instruments,
        ["instruments", _] => Endpoint::InstrumentsExchange,
        ["quote"] => Endpoint::Quote,
        ["quote", "ohlc"] => Endpoint::QuoteOhlc,
        ["quote", "ltp"] => Endpoint::QuoteLtp,
        ["margins", "orders"] => Endpoint::MarginsOrders,
        ["margins", "basket"] => Endpoint::MarginsBasket,
        ["charges", "orders"] => Endpoint::ChargesOrders,
        ["session", "token"] => Endpoint::SessionToken,
        _ => Endpoint::Unknown,
    }
}

fn http_error(kind: HttpErrorKind, status: u16, method: Method, endpoint: Endpoint) -> HttpError {
    HttpError::new(
        kind,
        method,
        endpoint,
        crate::kite::error::TransportStage::ResponseReceived,
    )
    .with_status(status)
}

/// Classify an error response (or an error envelope on a 2xx status).
fn error_response(status: u16, body: &Value, method: Method, endpoint: Endpoint) -> HttpError {
    let obj = body.as_object();
    let text = |k: &str| obj.and_then(|o| o.get(k)).and_then(Value::as_str);
    let is_envelope = text("status") == Some("error")
        || obj.is_some_and(|o| o.contains_key("error_type") || o.contains_key("message"));
    if !is_envelope {
        return http_error(HttpErrorKind::HttpStatus, status, method, endpoint)
            .with_detail("the error response has no broker error envelope");
    }
    let broker = BrokerError::new(text("error_type"), text("message"));
    let token_rejected = matches!(
        broker.error_type(),
        Some(Inbound::Known(KiteApiException::TokenException))
    );
    let kind = if token_rejected || status == 403 {
        HttpErrorKind::AuthRejected
    } else {
        HttpErrorKind::Broker
    };
    http_error(kind, status, method, endpoint).with_broker(broker)
}

/// Total classification of a JSON response.
// Internal error path; the error is boxed into `ManjaError::Http`.
#[allow(clippy::result_large_err)]
pub(crate) fn classify_json<T: DeserializeOwned>(
    status: u16,
    body: &[u8],
    method: Method,
    endpoint: Endpoint,
) -> std::result::Result<KiteApiResponse<T>, HttpError> {
    let parsed: std::result::Result<Value, _> = serde_json::from_slice(body);
    if !(200..300).contains(&status) {
        return Err(match parsed {
            Ok(v) => error_response(status, &v, method, endpoint),
            Err(_) => http_error(HttpErrorKind::HttpStatus, status, method, endpoint)
                .with_detail(&format!("non-JSON error body of {} bytes", body.len())),
        });
    }
    let decode = |detail: String| {
        http_error(HttpErrorKind::Decode, status, method, endpoint).with_detail(&detail)
    };
    let value = parsed.map_err(|e| decode(format!("malformed JSON success body: {e}")))?;
    let Some(obj) = value.as_object() else {
        return Err(decode("the success body is not a JSON object".into()));
    };
    match obj.get("status").and_then(Value::as_str) {
        Some("success") => {}
        Some("error") => return Err(error_response(status, &value, method, endpoint)),
        _ => return Err(decode("the envelope lacks status = \"success\"".into())),
    }
    let data = match obj.get("data") {
        None | Some(Value::Null) => return Err(decode("the success envelope has no data".into())),
        Some(d) => d.clone(),
    };
    let data: T = serde_json::from_value(data).map_err(|e| {
        decode(format!(
            "the payload does not match the endpoint's type: {e}"
        ))
    })?;
    Ok(KiteApiResponse {
        status: "success".to_string(),
        data: Some(data),
        message: obj
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        error_type: None,
    })
}

/// Total classification of a text (CSV) response.
#[allow(clippy::result_large_err)]
fn classify_text(
    status: u16,
    body: &[u8],
    method: Method,
    endpoint: Endpoint,
) -> std::result::Result<String, HttpError> {
    if !(200..300).contains(&status) {
        let err = match serde_json::from_slice::<Value>(body) {
            Ok(v) => error_response(status, &v, method, endpoint),
            Err(_) => http_error(HttpErrorKind::HttpStatus, status, method, endpoint)
                .with_detail(&format!("non-JSON error body of {} bytes", body.len())),
        };
        return Err(err);
    }
    String::from_utf8(body.to_vec()).map_err(|_| {
        http_error(HttpErrorKind::Decode, status, method, endpoint)
            .with_detail("the text body is not valid UTF-8")
    })
}

#[cfg(test)]
pub mod test_utils {
    use std::collections::HashMap;

    use super::*;
    use crate::kite::connect::credentials::KiteCredentials;

    use mockito::ServerGuard;

    /// Fixture loading, shared with the integration test support crate.
    pub mod fixtures {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/fixtures.rs"
        ));
    }

    /// Deserialize the `data` field of the official fixture `name`.
    ///
    /// Panics with the resolved fixture path if the file is missing, malformed,
    /// or has no `data` field.
    pub fn read_to_object<M>(name: &str) -> M
    where
        M: DeserializeOwned,
    {
        let response: KiteApiResponse<M> = fixtures::json(name).unwrap();
        response.data.unwrap_or_else(|| {
            panic!(
                "fixture {} has no `data` field",
                fixtures::mocks_dir().join(name).display()
            )
        })
    }

    pub type HTTPMethod = &'static str;
    pub type APIEndpoint = &'static str;
    /// Official fixture file name, served unchanged as the response body.
    pub type TestResponse = &'static str;

    pub async fn add_mocks(
        mut server: ServerGuard,
        mock_map: HashMap<(HTTPMethod, APIEndpoint), TestResponse>,
    ) -> ServerGuard {
        let mut mocks = Vec::new();
        for ((method, api_endpoint), fixture) in mock_map {
            let response_json = fixtures::json_body(fixture).unwrap();
            let m = server
                .mock(method, api_endpoint)
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(response_json)
                .create_async();
            mocks.push(m)
        }
        let _ms = futures::future::join_all(mocks).await;
        server
    }

    /// A client pointed at its own mock server through per-test configuration.
    ///
    /// Reads no `.env` file and mutates no process environment, so tests stay
    /// independent when run in parallel.
    pub async fn get_manja_test_client() -> (ServerGuard, HTTPClient) {
        let server = mockito::Server::new_async().await;
        let config = Config::from_parts(
            server.url(),
            server.url(),
            server.url(),
            KiteCredentials::new(
                "test_api_key",
                "test_api_secret",
                "test_user_id",
                "test_user_pwd",
                "test_totp_key",
            ),
        );
        let session = read_to_object::<UserSession>("generate_session.json");

        (
            server,
            HTTPClient::with_config(config)
                .unwrap()
                .with_user_session(session)
                .unwrap(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_replacements_and_shared_builders_share_one_budget() {
        let a = HTTPClient::new(Config::default()).unwrap();
        let clone = a.clone();
        let replacement = a.with_credentials(Credentials::new("k", "t").unwrap());
        assert!(a.admission().same_scope(clone.admission()));
        assert!(a.admission().same_scope(replacement.admission()));
        let shared = HTTPClient::builder(Config::default())
            .admission(a.admission().clone())
            .build()
            .unwrap();
        assert!(a.admission().same_scope(shared.admission()));
        let independent = HTTPClient::new(Config::default()).unwrap();
        assert!(!a.admission().same_scope(independent.admission()));
    }

    #[test]
    fn modifications_are_keyed_by_their_order_id() {
        assert_eq!(
            rate_key(
                Method::Put,
                Endpoint::OrdersVarietyId,
                "/orders/regular/151220000000000"
            ),
            (
                RateClass::OrderModification,
                Some("151220000000000".to_string())
            )
        );
        assert_eq!(
            rate_key(
                Method::Delete,
                Endpoint::OrdersVarietyId,
                "/orders/regular/1"
            ),
            (RateClass::Standard, None)
        );
    }

    #[test]
    fn endpoint_templates_hide_dynamic_segments() {
        use Method::*;
        let cases = [
            (Get, "/user/margins/equity", Endpoint::UserMarginsSegment),
            (Post, "/orders/regular", Endpoint::OrdersVariety),
            (Get, "/orders/151220000000000", Endpoint::OrdersId),
            (
                Put,
                "/orders/regular/151220000000000",
                Endpoint::OrdersVarietyId,
            ),
            (Delete, "/orders/amo/1", Endpoint::OrdersVarietyId),
            (Get, "/orders/1/trades", Endpoint::OrdersIdTrades),
            (Get, "/instruments/NSE", Endpoint::InstrumentsExchange),
            (
                Post,
                "/margins/basket?consider_positions=true",
                Endpoint::MarginsBasket,
            ),
            (Delete, "/session/token", Endpoint::SessionToken),
            (Get, "/gtt/triggers", Endpoint::Unknown),
        ];
        for (m, path, expected) in cases {
            assert_eq!(endpoint_template(m, path), expected, "{path}");
        }
    }
}
