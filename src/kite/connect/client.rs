//! Asynchronous HTTP client.
//!
//! [`HTTPClient`] owns a pooled HTTP transport, its [`Config`], a shared
//! [`Admission`] scope, and an optional immutable [`Credentials`] snapshot,
//! and hands out the resource facades (`user()`, `orders()`, …). Cloning a
//! client shares its transport; [`HTTPClient::with_credentials`] builds a
//! client for different credentials on the same transport and budget scope
//! without changing the original.
//!
//! Every operation runs under the scheduler
//! ([`crate::kite::connect::scheduler`]): a total deadline covering
//! admission, attempts and backoff, an attempt timeout, and an
//! endpoint-class retry policy under which placement, modification,
//! cancellation, position conversion and the session operations make at most
//! one actual attempt.
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
//! # Cancellation
//!
//! Operation futures are lazy: dropping one before it is first polled sends
//! nothing. Dropping one while it waits for admission or before dispatch
//! cancels local work only. Dropping one after dispatch does not cancel
//! anything at the broker; the request may still be processed. Either way
//! the client records a `Cancelled` entry with the stage reached in
//! [`HTTPClient::diagnostics`].
//!
use core::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
        scheduler::{AttemptCtx, DispatchPermit, OpSpec, PermitTarget, Scheduler},
    },
    error::{
        BrokerError, HttpError, HttpErrorKind, KiteApiException, Result, TransportStage as Stage,
    },
    obs::diagnostics::{BoundedText, FailureHistory, DEFAULT_HISTORY},
    obs::schema::{Endpoint, Method},
    protocol::Inbound,
    traits::KiteConfig,
};

struct Transport {
    http: reqwest::Client,
    config: Config,
    admission: Admission,
    scheduler: Scheduler,
    in_flight: Arc<tokio::sync::Semaphore>,
    failures: Mutex<FailureHistory<HttpFailure>>,
    origin: Instant,
}

/// One recorded failure or cancellation of an HTTP operation.
///
/// It carries no URL, body, header or credential; the message is the
/// broker's, bounded and sanitized.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HttpFailure {
    /// HTTP method.
    pub method: Method,
    /// Endpoint template.
    pub endpoint: Endpoint,
    /// Error category.
    pub kind: HttpErrorKind,
    /// HTTP status, if a response was received.
    pub http_status: Option<u16>,
    /// Stage reached.
    pub stage: Stage,
    /// Broker `error_type` text, if any.
    pub broker_error_type: Option<String>,
    /// Broker message, bounded and sanitized.
    pub message: Option<BoundedText>,
    /// Attempt number of the failure.
    pub attempt: u32,
    /// Monotonic time of the failure since the transport was created.
    pub at: Duration,
}

/// A bounded diagnostics snapshot of one transport. Available without any
/// telemetry consumer.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HttpDiagnostics {
    /// Transport attempts in progress.
    pub active_attempts: usize,
    /// Operations waiting for admission capacity in the scope.
    pub admission_waiters: usize,
    /// The most recent failures and cancellations, oldest first
    /// (`B-DIAG-01`).
    pub last_failures: Vec<HttpFailure>,
    /// Incremented on every recorded failure, so a stale snapshot is
    /// detectable.
    pub snapshot_revision: u64,
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

// Records an operation's outcome in the transport diagnostics, including
// cancellation when its future is dropped before completing.
struct OpGuard<'a> {
    transport: &'a Transport,
    method: Method,
    endpoint: Endpoint,
    dispatched: Arc<AtomicBool>,
    done: bool,
}

impl OpGuard<'_> {
    fn finish<T>(
        mut self,
        result: std::result::Result<T, HttpError>,
    ) -> std::result::Result<T, HttpError> {
        self.done = true;
        if let Err(e) = &result {
            self.transport.record(HttpFailure {
                method: e.method(),
                endpoint: e.endpoint(),
                kind: e.kind(),
                http_status: e.http_status(),
                stage: e.stage(),
                broker_error_type: e
                    .broker()
                    .and_then(|b| b.error_type())
                    .map(|t| t.as_wire().to_string()),
                message: e.broker().and_then(|b| b.message()).cloned(),
                attempt: e.attempt(),
                at: self.transport.origin.elapsed(),
            });
        }
        result
    }
}

impl Drop for OpGuard<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // Dropped mid-operation: record what the SDK knows, and nothing
        // about whether the broker acted.
        let stage = if self.dispatched.load(Ordering::Acquire) {
            Stage::Started
        } else {
            Stage::NotStarted
        };
        self.transport.record(HttpFailure {
            method: self.method,
            endpoint: self.endpoint,
            kind: HttpErrorKind::Cancelled,
            http_status: None,
            stage,
            broker_error_type: None,
            message: None,
            attempt: 0,
            at: self.transport.origin.elapsed(),
        });
    }
}

impl Transport {
    fn record(&self, failure: HttpFailure) {
        self.failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(failure);
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
        // Attempt timeouts are enforced by the scheduler, not the transport.
        let http = reqwest::ClientBuilder::new().build().map_err(|e| {
            HttpError::new(
                HttpErrorKind::Configuration,
                Method::Get,
                Endpoint::Unknown,
                Stage::NotStarted,
            )
            .with_detail("the HTTP transport could not be built")
            .with_source(e.without_url())
        })?;
        let in_flight = Arc::new(tokio::sync::Semaphore::new(config.limits().in_flight()));
        let scheduler = Scheduler::new(config.limits().scheduler().clone());
        Ok(Self {
            transport: Arc::new(Transport {
                http,
                config,
                admission,
                scheduler,
                in_flight,
                failures: Mutex::new(FailureHistory::new(DEFAULT_HISTORY)),
                origin: Instant::now(),
            }),
            credentials: None,
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

    /// HTTP configurations and Kite user credentials.
    ///
    pub fn http_config(&self) -> &Config {
        &self.transport.config
    }

    /// Admit capacity for `target` now, returning a [`DispatchPermit`] to
    /// pass to the matching operation.
    ///
    /// This is the explicit point between admission and dispatch: the
    /// caller may run its own checks before using the permit. The permit
    /// expires after `B-HTTP-12`; dropping it unused returns its capacity.
    pub async fn admit(&self, target: PermitTarget) -> Result<DispatchPermit> {
        Ok(self
            .transport
            .scheduler
            .admit(&self.transport.admission, target)
            .await?)
    }

    /// A bounded diagnostics snapshot of this client's transport.
    pub fn diagnostics(&self) -> HttpDiagnostics {
        let failures = self
            .transport
            .failures
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        HttpDiagnostics {
            active_attempts: self.transport.config.limits().in_flight()
                - self.transport.in_flight.available_permits(),
            admission_waiters: self.transport.admission.waiters(),
            last_failures: failures.entries(),
            snapshot_revision: failures.revision(),
        }
    }

    // --- [ API Groups ] ---

    /// To call [User] related APIs using this client.
    ///
    pub fn user(&self) -> User<'_> {
        User::new(self)
    }

    /// The session operations (token exchange and invalidation) for
    /// `api_key`. This client needs no credentials for them, and its own
    /// credentials are never attached to them.
    pub fn session(&self, api_key: ApiKey) -> Session<'_> {
        Session::new(self, api_key)
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
    pub(crate) async fn get_raw(&self, path: &str) -> Result<String> {
        let (m, endpoint) = labels(&reqwest::Method::GET, path);
        let guard = self.guard(m, endpoint);
        let result = async {
            let (status, body, attempt) = self
                .execute(
                    reqwest::Method::GET,
                    path,
                    BodyKind::Csv,
                    None,
                    true,
                    &guard.dispatched,
                    |rb| rb,
                )
                .await?;
            classify_text(status, &body, m, endpoint).map_err(|e| e.with_attempt(attempt))
        }
        .await;
        Ok(guard.finish(result)?)
    }

    /// GET `path` and decode the response envelope.
    pub(crate) async fn get<Model>(&self, path: &str) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        self.json(reqwest::Method::GET, path, None, true, |rb| rb)
            .await
    }

    /// GET `path` with a query and decode the response envelope.
    pub(crate) async fn get_with_query<Q, Model>(
        &self,
        path: &str,
        query: &Q,
    ) -> Result<KiteApiResponse<Model>>
    where
        Q: Serialize + ?Sized,
        Model: DeserializeOwned,
    {
        self.json(reqwest::Method::GET, path, None, true, |rb| rb.query(query))
            .await
    }

    /// Send `pairs` form-encoded (`application/x-www-form-urlencoded`,
    /// `kite-api-docs/docs/connect/v3/response-structure.md:2`) and decode the
    /// response envelope. `validate` runs first: an invalid request, or a
    /// body over `B-HTTP-09`, fails before admission and sends nothing.
    pub(crate) async fn send_form<Model>(
        &self,
        method: reqwest::Method,
        path: &str,
        validate: std::result::Result<(), crate::kite::connect::models::RequestError>,
        pairs: Vec<(&'static str, String)>,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        let (m, endpoint) = labels(&method, path);
        let rejected = |detail: &str| {
            HttpError::new(HttpErrorKind::Validation, m, endpoint, Stage::NotStarted)
                .with_detail(detail)
        };
        validate.map_err(|e| rejected(&e.to_string()))?;
        let body = form_urlencode(&pairs);
        if body.len() > self.transport.config.limits().request_body_bytes() {
            return Err(rejected("the request body exceeds its bound").into());
        }
        self.json(method, path, permit, true, |rb| {
            if pairs.is_empty() {
                // Nothing to send: no body and no content type.
                rb
            } else {
                rb.header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(body.clone())
            }
        })
        .await
    }

    /// Send `body` as JSON (the margin and charges endpoints,
    /// `kite-api-docs/docs/connect/v3/margins.md:13`) and decode the response
    /// envelope. `validate` runs first: an invalid request, or a body over
    /// `B-HTTP-09`, fails before admission and sends nothing.
    pub(crate) async fn send_json<Model, T>(
        &self,
        method: reqwest::Method,
        path: &str,
        validate: std::result::Result<(), crate::kite::connect::models::RequestError>,
        body: &T,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        T: Serialize + ?Sized,
    {
        let (m, endpoint) = labels(&method, path);
        let rejected = |detail: &str| {
            HttpError::new(HttpErrorKind::Validation, m, endpoint, Stage::NotStarted)
                .with_detail(detail)
        };
        validate.map_err(|e| rejected(&e.to_string()))?;
        let body = serde_json::to_vec(body)
            .map_err(|_| rejected("the request body could not be serialized"))?;
        if body.len() > self.transport.config.limits().request_body_bytes() {
            return Err(rejected("the request body exceeds its bound").into());
        }
        self.json(method, path, None, true, |rb| {
            rb.header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.clone())
        })
        .await
    }

    /// Form-encoded request without the client's `Authorization` header,
    /// for the session operations.
    pub(crate) async fn send_form_unauthenticated<Model>(
        &self,
        method: reqwest::Method,
        path: &str,
        pairs: Vec<(&'static str, String)>,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        let body = form_urlencode(&pairs);
        drop(pairs);
        self.json(method, path, None, false, |rb| {
            rb.header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body.clone())
        })
        .await
    }

    /// DELETE with query parameters and without the client's
    /// `Authorization` header, for session invalidation.
    pub(crate) async fn delete_unauthenticated<Model>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
    {
        self.json(reqwest::Method::DELETE, path, None, false, |rb| {
            rb.query(query)
        })
        .await
    }

    fn guard(&self, method: Method, endpoint: Endpoint) -> OpGuard<'_> {
        OpGuard {
            transport: &self.transport,
            method,
            endpoint,
            dispatched: Arc::new(AtomicBool::new(false)),
            done: false,
        }
    }

    pub(crate) async fn json<Model, B>(
        &self,
        method: reqwest::Method,
        path: &str,
        permit: Option<DispatchPermit>,
        auth: bool,
        build: B,
    ) -> Result<KiteApiResponse<Model>>
    where
        Model: DeserializeOwned,
        B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    {
        let (m, endpoint) = labels(&method, path);
        let guard = self.guard(m, endpoint);
        let result = async {
            let (status, body, attempt) = self
                .execute(
                    method.clone(),
                    path,
                    BodyKind::Json,
                    permit,
                    auth,
                    &guard.dispatched,
                    build,
                )
                .await?;
            classify_json(status, &body, m, endpoint).map_err(|e| e.with_attempt(attempt))
        }
        .await;
        Ok(guard.finish(result)?)
    }

    /// Run the operation under the scheduler and return the final status,
    /// body and attempt number. Non-2xx statuses become errors here so the
    /// scheduler can decide on retries.
    #[allow(clippy::too_many_arguments)]
    async fn execute<B>(
        &self,
        method: reqwest::Method,
        path: &str,
        kind: BodyKind,
        permit: Option<DispatchPermit>,
        auth: bool,
        op_dispatched: &Arc<AtomicBool>,
        build: B,
    ) -> std::result::Result<(u16, Vec<u8>, u32), HttpError>
    where
        B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    {
        let (m, endpoint) = labels(&method, path);
        let (_, order_id) = rate_key(m, endpoint, path);
        let spec = OpSpec {
            method: m,
            endpoint,
            order_id: order_id.as_deref(),
        };
        let build = &build;
        let method = &method;
        self.transport
            .scheduler
            .run(spec, &self.transport.admission, permit, |ctx| async move {
                let number = ctx.number;
                let (status, body) = self
                    .attempt(
                        method,
                        path,
                        kind,
                        auth,
                        build,
                        m,
                        endpoint,
                        ctx,
                        op_dispatched,
                    )
                    .await?;
                if !(200..300).contains(&status) {
                    // Classify now so the scheduler sees the status.
                    return Err(classify_json::<Value>(status, &body, m, endpoint)
                        .err()
                        .unwrap_or_else(|| {
                            http_error(HttpErrorKind::HttpStatus, status, m, endpoint)
                        })
                        .with_attempt(number));
                }
                Ok((status, body, number))
            })
            .await
    }

    /// One transport attempt: build, dispatch and read a bounded body.
    #[allow(clippy::too_many_arguments)]
    async fn attempt<B>(
        &self,
        method: &reqwest::Method,
        path: &str,
        kind: BodyKind,
        auth: bool,
        build: &B,
        m: Method,
        endpoint: Endpoint,
        ctx: AttemptCtx,
        op_dispatched: &AtomicBool,
    ) -> std::result::Result<(u16, Vec<u8>), HttpError>
    where
        B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    {
        let attempt = ctx.number;
        let mut token = ctx.token;
        let not_started = |kind: HttpErrorKind, detail: &str| {
            HttpError::new(kind, m, endpoint, Stage::NotStarted)
                .with_attempt(attempt)
                .with_detail(detail)
        };
        let _in_flight = self
            .transport
            .in_flight
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| not_started(HttpErrorKind::Admission, "the transport is closed"))?;
        let url = self.transport.config.url(path);
        let mut rb = self
            .transport
            .http
            .request(method.clone(), url)
            .header("X-Kite-Version", "3");
        if let (true, Some(creds)) = (auth, &self.credentials) {
            let mut value =
                HeaderValue::from_str(creds.authorization_header().expose()).map_err(|_| {
                    not_started(
                        HttpErrorKind::Configuration,
                        "the Authorization header could not be built",
                    )
                })?;
            value.set_sensitive(true);
            rb = rb.header(AUTHORIZATION, value);
        }
        let request = build(rb).build().map_err(|e| {
            not_started(
                HttpErrorKind::Configuration,
                "the request could not be built",
            )
            .with_source(e.without_url())
        })?;
        // Dispatch now: the admitted capacity is used, and from here on the
        // broker may receive the request.
        token.dispatch();
        op_dispatched.store(true, Ordering::Release);
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

/// Encode form fields as `application/x-www-form-urlencoded`: ASCII letters,
/// digits and `*-._` are kept, a space becomes `+`, and every other byte is
/// percent-encoded.
pub(crate) fn form_urlencode(pairs: &[(&str, String)]) -> String {
    fn enc(out: &mut String, s: &str) {
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                    out.push(b as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
    }
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        enc(&mut out, k);
        out.push('=');
        enc(&mut out, v);
    }
    out
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
