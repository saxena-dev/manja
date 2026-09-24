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
//!   type (`kite:response-structure.md:15`). The one exception is the
//!   mutual fund SIP list, documented without a `status`
//!   (`kite:mutual-funds.md:219-221`): there an absent `status` is accepted,
//!   while any `status` other than `success` is still not;
//! - a non-2xx status or `status = "error"` is never `Ok`, whether or not
//!   `error_type` is present or known (`kite:response-structure.md:28`);
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
//! # Observability
//!
//! A client records in the [`Observability`] scope it was built with
//! ([`HttpClientBuilder::observability`], [`HTTPClient::with_observability`]),
//! or in a disabled scope. Clones and derived clients share it. Each polled
//! operation gets one `manja.http.operation` span and one completion count
//! and duration, covering admission, attempts and backoff; an operation
//! rejected before admission counts with result `validation`, and an
//! unpolled future records nothing. Each admission wait gets its own
//! `manja.http.admission` span and wait histogram. Each dispatched attempt
//! gets a child `manja.http.attempt` span, a count and a duration that
//! excludes admission and backoff; a failure before dispatch makes no
//! attempt, and a retry is counted only when another attempt is dispatched.
//! The in-flight and waiter gauges are held by guards, so they settle on
//! success, error and cancellation. Context is passed explicitly through
//! the scheduler, never through a task-local or global. Labels are endpoint
//! templates and closed outcome values only.
//!
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tracing::{Instrument as _, Span};

use crate::kite::{
    connect::{
        admission::{Admission, RateClass},
        api::{Charges, Gtt, Margins, Market, MutualFunds, Orders, Portfolio, Session, User},
        config::Config,
        credentials::{AccessToken, ApiKey, Credentials},
        models::{KiteApiResponse, UserSession},
        scheduler::{
            AttemptCtx, DispatchPermit, OpObs, OpSpec, PermitTarget, RetryClass, Scheduler,
        },
    },
    error::{
        BrokerError, HttpError, HttpErrorKind, KiteApiException, ManjaError, Result,
        TransportStage as Stage,
    },
    obs::diagnostics::{BoundedText, FailureHistory, DEFAULT_HISTORY},
    obs::handle::{Labels, Observability},
    obs::schema::{Endpoint, HttpOperationResult, Instrument, Method, QuotaClass},
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
    obs: Observability,
    // This transport's operations waiting for admission, per quota class.
    waiting: [AtomicUsize; 4],
}

fn quota_index(q: QuotaClass) -> usize {
    QuotaClass::ALL.iter().position(|c| *c == q).unwrap_or(0)
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
    /// This transport's operations waiting for admission, per quota class,
    /// in [`QuotaClass::ALL`] order.
    pub admission_waiters_by_class: Vec<(QuotaClass, usize)>,
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
    observability: Option<Observability>,
}

impl HttpClientBuilder {
    /// Record spans and metrics in `obs`'s scope. Without it the client
    /// uses [`Observability::disabled`]; its typed diagnostics work either
    /// way.
    pub fn observability(mut self, obs: Observability) -> Self {
        self.observability = Some(obs);
        self
    }

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
        let mut client = HTTPClient::build_transport(
            self.config,
            self.admission.unwrap_or_default(),
            self.observability.unwrap_or_default(),
        )?;
        client.credentials = self.credentials;
        Ok(client)
    }
}

// One polled logical operation: its span, its completion count and
// duration, and its entry in the transport diagnostics. Dropped before
// finishing, the operation was cancelled.
struct OpGuard<'a> {
    transport: &'a Transport,
    method: Method,
    endpoint: Endpoint,
    quota: QuotaClass,
    span: Span,
    operation_id: u64,
    started: tokio::time::Instant,
    dispatched: Arc<AtomicBool>,
    done: bool,
}

impl OpGuard<'_> {
    fn finish<T>(
        mut self,
        result: std::result::Result<T, HttpError>,
    ) -> std::result::Result<T, HttpError> {
        self.done = true;
        match &result {
            Ok(_) => self.complete(HttpOperationResult::Ok, Stage::ResponseReceived),
            Err(e) => {
                self.complete(operation_result(e), e.stage());
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
        }
        result
    }

    fn complete(&self, result: HttpOperationResult, stage: Stage) {
        let obs = &self.transport.obs;
        let labels = |i| Labels::http_operation(i, self.method, self.endpoint, self.quota, result);
        obs.counter(labels(Instrument::HttpOperationsTotal), 1);
        obs.histogram(
            labels(Instrument::HttpOperationDuration),
            self.started.elapsed().as_secs_f64(),
        );
        self.span.record("result", result.as_str());
        self.span.record("stage", stage.as_str());
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
        self.complete(HttpOperationResult::Cancelled, stage);
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

/// The normalized result of a finished operation.
fn operation_result(e: &HttpError) -> HttpOperationResult {
    match e.kind() {
        HttpErrorKind::Validation | HttpErrorKind::Configuration => HttpOperationResult::Validation,
        HttpErrorKind::Admission => HttpOperationResult::AdmissionRejected,
        HttpErrorKind::Deadline => HttpOperationResult::Deadline,
        HttpErrorKind::HttpStatus => HttpOperationResult::HttpStatus,
        HttpErrorKind::Broker => HttpOperationResult::BrokerError,
        HttpErrorKind::AuthRejected => HttpOperationResult::AuthRejected,
        HttpErrorKind::Decode => HttpOperationResult::DecodeError,
        HttpErrorKind::Cancelled => HttpOperationResult::Cancelled,
        _ if e.is_timeout() => HttpOperationResult::Timeout,
        _ => HttpOperationResult::TransportError,
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
            observability: None,
        }
    }

    /// A client for `config` with no credentials and its own budget scope.
    ///
    /// Fails with a configuration error if the HTTP transport cannot be
    /// built.
    pub fn new(config: Config) -> Result<Self> {
        Self::builder(config).build()
    }

    /// A client for `config` recording in `obs`'s scope, with no
    /// credentials and its own budget scope.
    pub fn with_observability(config: Config, obs: Observability) -> Result<Self> {
        Self::builder(config).observability(obs).build()
    }

    fn build_transport(config: Config, admission: Admission, obs: Observability) -> Result<Self> {
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
                obs,
                waiting: Default::default(),
            }),
            credentials: None,
        })
    }

    /// The observability scope this client records in.
    pub fn observability(&self) -> &Observability {
        &self.transport.obs
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

    /// The client's configuration: endpoints and limits.
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
            .admit(&self.transport.admission, target, &self.transport.obs)
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
            admission_waiters_by_class: QuotaClass::ALL
                .iter()
                .map(|q| {
                    let n = self.transport.waiting[quota_index(*q)].load(Ordering::Relaxed);
                    (*q, n)
                })
                .collect(),
            last_failures: failures.entries(),
            snapshot_revision: failures.revision(),
        }
    }

    // --- [ API Groups ] ---
    //
    // Every accessor borrows the client immutably: a client is a shared,
    // cloneable handle with no client-wide lock, so any number of
    // resources may be used at once, from any task.

    /// The user resource: profile and funds.
    pub fn user(&self) -> User<'_> {
        User::new(self)
    }

    /// The session operations (token exchange and invalidation) for
    /// `api_key`. This client needs no credentials for them, and its own
    /// credentials are never attached to them.
    pub fn session(&self, api_key: ApiKey) -> Session<'_> {
        Session::new(self, api_key)
    }

    /// The orders resource: placement, modification, cancellation and
    /// order and trade books.
    pub fn orders(&self) -> Orders<'_> {
        Orders::new(self)
    }

    /// The portfolio resource: holdings, positions, conversion and
    /// auctions.
    pub fn portfolio(&self) -> Portfolio<'_> {
        Portfolio::new(self)
    }

    /// The GTT resource: Good Till Triggered orders.
    pub fn gtt(&self) -> Gtt<'_> {
        Gtt::new(self)
    }

    /// The mutual fund resource: orders, SIPs, holdings and instruments.
    pub fn mutual_funds(&self) -> MutualFunds<'_> {
        MutualFunds::new(self)
    }

    /// The market resource: quotes and the instrument master.
    pub fn market(&self) -> Market<'_> {
        Market::new(self)
    }

    /// The margin calculations.
    pub fn margins(&self) -> Margins<'_> {
        Margins::new(self)
    }

    /// The order-charges calculation.
    pub fn charges(&self) -> Charges<'_> {
        Charges::new(self)
    }

    // --- [ HTTP verb functions ] ---

    /// GET `path` and return the raw (CSV) body.
    pub(crate) async fn get_raw(&self, path: &str) -> Result<String> {
        self.get_csv(path, |text| Ok(text.to_string())).await
    }

    /// GET `path` and parse its text (CSV) body with `parse`. A parse
    /// failure is the operation's result.
    pub(crate) async fn get_csv<T, P>(&self, path: &str, parse: P) -> Result<T>
    where
        P: Fn(&str) -> std::result::Result<T, HttpError>,
    {
        let (m, endpoint) = labels(&reqwest::Method::GET, path);
        self.operation(
            reqwest::Method::GET,
            path,
            BodyKind::Csv,
            None,
            true,
            |rb| rb,
            |status, body| classify_text(status, body, m, endpoint).and_then(|t| parse(&t)),
        )
        .await
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
    /// `kite:response-structure.md:2`) and decode the
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
            self.reject(
                HttpError::new(HttpErrorKind::Validation, m, endpoint, Stage::NotStarted)
                    .with_detail(detail),
            )
        };
        validate.map_err(|e| rejected(&e.to_string()))?;
        let body = form_urlencode(&pairs);
        if body.len() > self.transport.config.limits().request_body_bytes() {
            return Err(rejected("the request body exceeds its bound"));
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
    /// `kite:margins.md:13`) and decode the response
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
            self.reject(
                HttpError::new(HttpErrorKind::Validation, m, endpoint, Stage::NotStarted)
                    .with_detail(detail),
            )
        };
        validate.map_err(|e| rejected(&e.to_string()))?;
        let body = serde_json::to_vec(body)
            .map_err(|_| rejected("the request body could not be serialized"))?;
        if body.len() > self.transport.config.limits().request_body_bytes() {
            return Err(rejected("the request body exceeds its bound"));
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
        let class = RetryClass::of(method, endpoint);
        let quota = class.quota_class();
        let operation_id = self.transport.obs.next_operation_id();
        let deadline = self.transport.scheduler.limits().deadline(class);
        let span = tracing::debug_span!(
            "manja.http.operation",
            operation_id,
            method = method.as_str(),
            endpoint = endpoint.as_str(),
            quota_class = quota.as_str(),
            deadline_ms = deadline.as_millis().min(u64::MAX as u128) as u64,
            result = tracing::field::Empty,
            stage = tracing::field::Empty,
        );
        OpGuard {
            transport: &self.transport,
            method,
            endpoint,
            quota,
            span,
            operation_id,
            started: tokio::time::Instant::now(),
            dispatched: Arc::new(AtomicBool::new(false)),
            done: false,
        }
    }

    /// Complete an operation rejected before admission: it counts as one
    /// operation with no attempt.
    pub(crate) fn reject(&self, err: HttpError) -> ManjaError {
        let guard = self.guard(err.method(), err.endpoint());
        let _enter = guard.span.clone().entered();
        match guard.finish::<()>(Err(err)) {
            Err(e) => e.into(),
            Ok(()) => unreachable!("finish returns its input"),
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
        self.operation(
            method,
            path,
            BodyKind::Json,
            permit,
            auth,
            build,
            |status, body| classify_json(status, body, m, endpoint),
        )
        .await
    }

    /// Run one logical operation inside its span: every attempt's response
    /// is classified by `classify`, so the scheduler and the attempt
    /// accounting see the final result of each attempt.
    #[allow(clippy::too_many_arguments)]
    async fn operation<T, B, C>(
        &self,
        method: reqwest::Method,
        path: &str,
        kind: BodyKind,
        permit: Option<DispatchPermit>,
        auth: bool,
        build: B,
        classify: C,
    ) -> Result<T>
    where
        B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
        C: Fn(u16, &[u8]) -> std::result::Result<T, HttpError>,
    {
        let (m, endpoint) = labels(&method, path);
        let guard = self.guard(m, endpoint);
        let span = guard.span.clone();
        let result = self
            .execute(method, path, kind, permit, auth, &guard, build, classify)
            .instrument(span)
            .await;
        Ok(guard.finish(result)?)
    }

    /// Run the operation under the scheduler and return its classified
    /// result.
    #[allow(clippy::too_many_arguments)]
    async fn execute<T, B, C>(
        &self,
        method: reqwest::Method,
        path: &str,
        kind: BodyKind,
        permit: Option<DispatchPermit>,
        auth: bool,
        guard: &OpGuard<'_>,
        build: B,
        classify: C,
    ) -> std::result::Result<T, HttpError>
    where
        B: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
        C: Fn(u16, &[u8]) -> std::result::Result<T, HttpError>,
    {
        let (m, endpoint) = (guard.method, guard.endpoint);
        let (_, order_id) = rate_key(m, endpoint, path);
        let spec = OpSpec {
            method: m,
            endpoint,
            order_id: order_id.as_deref(),
        };
        let ctx = OpObs {
            obs: &self.transport.obs,
            span: &guard.span,
            operation_id: guard.operation_id,
            waiting: &self.transport.waiting[quota_index(guard.quota)],
        };
        let (build, classify, method) = (&build, &classify, &method);
        let op_dispatched = &guard.dispatched;
        self.transport
            .scheduler
            .run(
                spec,
                &self.transport.admission,
                permit,
                &ctx,
                |ctx| async move {
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
                    classify(status, &body).map_err(|e| e.with_attempt(number))
                },
            )
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
        let span = token.dispatch();
        op_dispatched.store(true, Ordering::Release);
        self.receive(request, kind, m, endpoint, attempt, &span)
            .instrument(span.clone())
            .await
    }

    // The dispatched part of an attempt, inside its span.
    async fn receive(
        &self,
        request: reqwest::Request,
        kind: BodyKind,
        m: Method,
        endpoint: Endpoint,
        attempt: u32,
        span: &Span,
    ) -> std::result::Result<(u16, Vec<u8>), HttpError> {
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
        span.record("http_status", status);
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
        ["portfolio", "holdings", "authorise"] => Endpoint::HoldingsAuthorise,
        ["portfolio", "positions"] => Endpoint::Positions,
        ["instruments"] => Endpoint::Instruments,
        ["instruments", _] => Endpoint::InstrumentsExchange,
        ["instruments", "historical", _, _] => Endpoint::InstrumentsHistorical,
        ["quote"] => Endpoint::Quote,
        ["quote", "ohlc"] => Endpoint::QuoteOhlc,
        ["quote", "ltp"] => Endpoint::QuoteLtp,
        ["margins", "orders"] => Endpoint::MarginsOrders,
        ["margins", "basket"] => Endpoint::MarginsBasket,
        ["charges", "orders"] => Endpoint::ChargesOrders,
        ["session", "token"] => Endpoint::SessionToken,
        ["mf", "orders"] => Endpoint::MfOrders,
        ["mf", "orders", _] => Endpoint::MfOrdersId,
        ["mf", "sips"] => Endpoint::MfSips,
        ["mf", "holdings"] => Endpoint::MfHoldings,
        ["mf", "instruments"] => Endpoint::MfInstruments,
        ["gtt", "triggers"] => Endpoint::GttTriggers,
        ["gtt", "triggers", _] => Endpoint::GttTriggersId,
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
    match (obj.get("status"), endpoint) {
        (Some(s), _) if s.as_str() == Some("success") => {}
        (Some(s), _) if s.as_str() == Some("error") => {
            return Err(error_response(status, &value, method, endpoint))
        }
        // The documented SIP list, like its official sample, is `{"data":
        // [...]}` with no status (`kite:mutual-funds.md:219-221`). Only that
        // endpoint may omit the status, and only by omitting the key.
        (None, Endpoint::MfSips) => {}
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
    fn only_the_sip_list_may_omit_the_envelope_status() {
        let bare = br#"{"data": []}"#;
        assert!(classify_json::<Vec<Value>>(200, bare, Method::Get, Endpoint::MfSips).is_ok());
        let e =
            classify_json::<Vec<Value>>(200, bare, Method::Get, Endpoint::MfHoldings).unwrap_err();
        assert_eq!(e.kind(), HttpErrorKind::Decode);
        let failed =
            br#"{"status": "error", "message": "m", "error_type": "InputException", "data": null}"#;
        assert!(classify_json::<Vec<Value>>(200, failed, Method::Get, Endpoint::MfSips).is_err());
        let odd = br#"{"status": "pending", "data": []}"#;
        assert!(classify_json::<Vec<Value>>(200, odd, Method::Get, Endpoint::MfSips).is_err());
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
                Get,
                "/instruments/historical/5633/minute?from=x",
                Endpoint::InstrumentsHistorical,
            ),
            (
                Post,
                "/margins/basket?consider_positions=true",
                Endpoint::MarginsBasket,
            ),
            (Delete, "/session/token", Endpoint::SessionToken),
            (Post, "/gtt/triggers", Endpoint::GttTriggers),
            (Delete, "/gtt/triggers/123", Endpoint::GttTriggersId),
            (Get, "/mf/orders/2b6ad4b7-c84e", Endpoint::MfOrdersId),
            (Get, "/mf/sips/", Endpoint::MfSips),
            (
                Post,
                "/portfolio/holdings/authorise",
                Endpoint::HoldingsAuthorise,
            ),
            (Get, "/alerts", Endpoint::Unknown),
        ];
        for (m, path, expected) in cases {
            assert_eq!(endpoint_template(m, path), expected, "{path}");
        }
    }
}
