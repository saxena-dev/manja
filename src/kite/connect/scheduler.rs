//! Operation scheduling: total deadlines, safe retries and dispatch permits.
//!
//! Admission ([`crate::kite::connect::admission`]) decides when an attempt
//! may start. The scheduler decides whether another attempt may be made and
//! bounds the whole operation:
//!
//! - **Total deadline** (`B-HTTP-01`, or `B-HTTP-11` for session
//!   operations) covers admission waits, attempts and backoff. When it
//!   expires the operation fails with a `Deadline` error.
//! - **Attempt timeout** (`B-HTTP-02`) bounds one transport attempt, and is
//!   itself cut short by the remaining deadline.
//! - **Retry policy** depends on the endpoint's [`RetryClass`]:
//!
//! | Class | Endpoints | Attempts | Justification |
//! |---|---|---|---|
//! | `Read` | every `GET` | up to `B-HTTP-03` (3) | idempotent reads |
//! | `Calc` | `POST /margins/orders`, `/margins/basket`, `/charges/orders` | up to `B-HTTP-03` (3) | documented as calculations with no order or position effect (`kite-api-docs/docs/connect/v3/margins.md:1-13,345-350`) |
//! | `Mut` | place, modify, cancel, position conversion, and any other non-`GET` endpoint without a documented class | exactly 1 (`B-HTTP-04`) | may change orders or positions |
//! | `Sess` | token exchange, session invalidation | exactly 1 (`B-HTTP-04`) | a request token is single-use; invalidation is not repeated implicitly |
//!
//! A `Read` or `Calc` attempt is retried only after HTTP 429, 502, 503 or
//! 504, a transport failure or an attempt timeout, with capped exponential
//! backoff and full jitter (`B-HTTP-05`), and never past the deadline.
//! `Mut` and `Sess` operations make at most one actual attempt per
//! invocation, including after a 429 or a lost response; any further
//! attempt is a new explicit call by the caller.
//!
//! # Dispatch permits
//!
//! A [`DispatchPermit`] is admitted capacity for one specific operation,
//! obtained ahead of time with `HTTPClient::admit`. It gives the caller an
//! explicit point between admission and dispatch, with no callback: run any
//! checks, then pass the permit to the operation. A permit is valid for
//! `B-HTTP-12` (1 s by default), belongs to one admission scope and one
//! target, and is consumed by value, so it cannot be reused. An expired,
//! wrong-scope or mismatched permit fails before any transport starts, and a
//! valid one skips admission instead of queueing again.
//!
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::Instant;
use tracing::{Instrument as _, Span};

use crate::kite::connect::admission::{Admission, AdmissionError, AdmissionGrant, RateClass};
use crate::kite::error::{HttpError, HttpErrorKind, RetryInfo, TransportStage};
use crate::kite::obs::handle::{GaugeGuard, Labels, Observability};
use crate::kite::obs::schema::{
    AdmissionResult, Endpoint, HttpAttemptResult, Instrument, Method, QuotaClass, RetryCause,
    TransportLabel,
};

/// Retry class of an endpoint (see the module table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RetryClass {
    /// Idempotent read.
    Read,
    /// Read-like calculation with no trading side effect.
    Calc,
    /// Trading mutation: one attempt.
    Mut,
    /// Session operation: one attempt.
    Sess,
}

impl RetryClass {
    /// The class of an HTTP operation.
    pub fn of(method: Method, endpoint: Endpoint) -> Self {
        match (method, endpoint) {
            (_, Endpoint::SessionToken) => Self::Sess,
            (Method::Get, _) => Self::Read,
            (
                Method::Post,
                Endpoint::MarginsOrders | Endpoint::MarginsBasket | Endpoint::ChargesOrders,
            ) => Self::Calc,
            _ => Self::Mut,
        }
    }

    /// The metric label of the class.
    pub fn quota_class(self) -> QuotaClass {
        match self {
            Self::Read => QuotaClass::Read,
            Self::Calc => QuotaClass::Calc,
            Self::Mut => QuotaClass::Mut,
            Self::Sess => QuotaClass::Sess,
        }
    }
}

/// Validated scheduling bounds (SDK contract §5.2).
///
/// | Bound | Default | Range | Constraint |
/// |---|---|---|---|
/// | `B-HTTP-01` operation deadline | 30 s | 1 s ..= 300 s | |
/// | `B-HTTP-02` attempt timeout | 10 s | 100 ms ..= 120 s | ≤ `B-HTTP-01` |
/// | `B-HTTP-03` read attempts | 3 | 1 ..= 5 | |
/// | `B-HTTP-04` mutation and session attempts | 1 | fixed | |
/// | `B-HTTP-05` backoff initial / cap | 250 ms / 5 s | 10 ms ..= 5 s / 10 ms ..= 30 s | cap ≥ initial |
/// | `B-HTTP-11` session deadline | 15 s | 1 s ..= 60 s | |
/// | `B-HTTP-12` permit validity | 1 s | 10 ms ..= 30 s | ≤ `B-HTTP-01` |
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchedulerLimits {
    operation_deadline: Duration,
    attempt_timeout: Duration,
    read_attempts: u32,
    backoff_initial: Duration,
    backoff_cap: Duration,
    session_deadline: Duration,
    permit_validity: Duration,
    jitter_seed: Option<u64>,
}

impl Default for SchedulerLimits {
    fn default() -> Self {
        Self {
            operation_deadline: Duration::from_secs(30),
            attempt_timeout: Duration::from_secs(10),
            read_attempts: 3,
            backoff_initial: Duration::from_millis(250),
            backoff_cap: Duration::from_secs(5),
            session_deadline: Duration::from_secs(15),
            permit_validity: Duration::from_secs(1),
            jitter_seed: None,
        }
    }
}

/// A scheduling bound was outside its range or broke a constraint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchedulerLimitError(pub &'static str);

impl fmt::Display for SchedulerLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is outside its permitted range", self.0)
    }
}

impl std::error::Error for SchedulerLimitError {}

fn within(
    bound: &'static str,
    v: Duration,
    min: Duration,
    max: Duration,
) -> Result<Duration, SchedulerLimitError> {
    if v < min || v > max {
        Err(SchedulerLimitError(bound))
    } else {
        Ok(v)
    }
}

impl SchedulerLimits {
    /// Set the operation deadline (`B-HTTP-01`).
    pub fn with_operation_deadline(mut self, d: Duration) -> Result<Self, SchedulerLimitError> {
        self.operation_deadline = within(
            "B-HTTP-01",
            d,
            Duration::from_secs(1),
            Duration::from_secs(300),
        )?;
        self.check()
    }

    /// Set the attempt timeout (`B-HTTP-02`).
    pub fn with_attempt_timeout(mut self, d: Duration) -> Result<Self, SchedulerLimitError> {
        self.attempt_timeout = within(
            "B-HTTP-02",
            d,
            Duration::from_millis(100),
            Duration::from_secs(120),
        )?;
        self.check()
    }

    /// Set the read attempt limit (`B-HTTP-03`).
    pub fn with_read_attempts(mut self, n: u32) -> Result<Self, SchedulerLimitError> {
        if !(1..=5).contains(&n) {
            return Err(SchedulerLimitError("B-HTTP-03"));
        }
        self.read_attempts = n;
        Ok(self)
    }

    /// Set the retry backoff initial delay and cap (`B-HTTP-05`).
    pub fn with_backoff(
        mut self,
        initial: Duration,
        cap: Duration,
    ) -> Result<Self, SchedulerLimitError> {
        self.backoff_initial = within(
            "B-HTTP-05 initial",
            initial,
            Duration::from_millis(10),
            Duration::from_secs(5),
        )?;
        self.backoff_cap = within(
            "B-HTTP-05 cap",
            cap,
            Duration::from_millis(10),
            Duration::from_secs(30),
        )?;
        self.check()
    }

    /// Set the session operation deadline (`B-HTTP-11`).
    pub fn with_session_deadline(mut self, d: Duration) -> Result<Self, SchedulerLimitError> {
        self.session_deadline = within(
            "B-HTTP-11",
            d,
            Duration::from_secs(1),
            Duration::from_secs(60),
        )?;
        Ok(self)
    }

    /// Set the dispatch permit validity (`B-HTTP-12`).
    pub fn with_permit_validity(mut self, d: Duration) -> Result<Self, SchedulerLimitError> {
        self.permit_validity = within(
            "B-HTTP-12",
            d,
            Duration::from_millis(10),
            Duration::from_secs(30),
        )?;
        self.check()
    }

    /// Seed the jitter generator, for reproducible schedules in tests.
    pub fn with_jitter_seed(mut self, seed: u64) -> Self {
        self.jitter_seed = Some(seed);
        self
    }

    fn check(self) -> Result<Self, SchedulerLimitError> {
        if self.attempt_timeout > self.operation_deadline {
            return Err(SchedulerLimitError("B-HTTP-02 <= B-HTTP-01"));
        }
        if self.backoff_cap < self.backoff_initial {
            return Err(SchedulerLimitError("B-HTTP-05 cap >= initial"));
        }
        if self.permit_validity > self.operation_deadline {
            return Err(SchedulerLimitError("B-HTTP-12 <= B-HTTP-01"));
        }
        Ok(self)
    }

    /// Operation deadline.
    pub fn operation_deadline(&self) -> Duration {
        self.operation_deadline
    }

    /// Attempt timeout.
    pub fn attempt_timeout(&self) -> Duration {
        self.attempt_timeout
    }

    /// Read attempt limit.
    pub fn read_attempts(&self) -> u32 {
        self.read_attempts
    }

    /// Backoff initial delay and cap.
    pub fn backoff(&self) -> (Duration, Duration) {
        (self.backoff_initial, self.backoff_cap)
    }

    /// Session operation deadline.
    pub fn session_deadline(&self) -> Duration {
        self.session_deadline
    }

    /// Permit validity.
    pub fn permit_validity(&self) -> Duration {
        self.permit_validity
    }

    /// Attempts allowed for `class`.
    pub fn max_attempts(&self, class: RetryClass) -> u32 {
        match class {
            RetryClass::Read | RetryClass::Calc => self.read_attempts,
            RetryClass::Mut | RetryClass::Sess => 1,
        }
    }

    /// Total deadline for `class`.
    pub fn deadline(&self, class: RetryClass) -> Duration {
        match class {
            RetryClass::Sess => self.session_deadline,
            _ => self.operation_deadline,
        }
    }
}

/// What a permit admits.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermitTarget {
    /// `POST /orders/{variety}`.
    PlaceOrder,
    /// `PUT /orders/{variety}/{order_id}` for this order.
    ModifyOrder {
        /// The order to modify.
        order_id: String,
    },
    /// `DELETE /orders/{variety}/{order_id}`.
    CancelOrder,
    /// `PUT /portfolio/positions`.
    ConvertPosition,
}

impl PermitTarget {
    pub(crate) fn operation(&self) -> (Method, Endpoint) {
        match self {
            Self::PlaceOrder => (Method::Post, Endpoint::OrdersVariety),
            Self::ModifyOrder { .. } => (Method::Put, Endpoint::OrdersVarietyId),
            Self::CancelOrder => (Method::Delete, Endpoint::OrdersVarietyId),
            Self::ConvertPosition => (Method::Put, Endpoint::Positions),
        }
    }

    fn order_id(&self) -> Option<&str> {
        match self {
            Self::ModifyOrder { order_id } => Some(order_id),
            _ => None,
        }
    }
}

/// Admitted capacity for one specific mutation, valid until it expires.
///
/// It is consumed by the operation it is passed to, so it can be used at
/// most once:
///
/// ```compile_fail
/// # async fn f(client: manja::kite::connect::client::HTTPClient,
/// #            permit: manja::kite::connect::scheduler::DispatchPermit) {
/// let a = permit;
/// let b = permit; // use of moved value
/// # }
/// ```
///
/// Dropping an unused permit returns its capacity to the admission scope.
#[must_use = "an unused permit returns its capacity when dropped"]
pub struct DispatchPermit {
    grant: AdmissionGrant,
    target: PermitTarget,
    expires_at: Instant,
}

impl fmt::Debug for DispatchPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DispatchPermit")
            .field("target", &self.target)
            .field("expired", &self.is_expired())
            .finish()
    }
}

impl DispatchPermit {
    /// What the permit admits.
    pub fn target(&self) -> &PermitTarget {
        &self.target
    }

    /// Whether the permit has expired.
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    /// Time left before the permit expires.
    pub fn remaining(&self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }

    /// Whether the permit was issued by `admission`.
    pub fn belongs_to(&self, admission: &Admission) -> bool {
        self.grant.belongs_to(admission)
    }
}

/// Hands admitted capacity to the transport at the moment of dispatch, and
/// records that dispatch happened.
///
/// Dispatch is also where an attempt starts to exist for observability: its
/// span, its in-flight gauge contribution and, for attempt n > 1, its retry
/// count. An attempt that fails before dispatch leaves no trace.
pub(crate) struct DispatchToken {
    grant: Option<AdmissionGrant>,
    dispatched: Arc<AtomicBool>,
    obs: AttemptObs,
    slot: Arc<Mutex<Option<AttemptRecord>>>,
}

// What the token needs to open the attempt's span and series.
struct AttemptObs {
    obs: Observability,
    parent: Span,
    operation_id: u64,
    number: u32,
    method: Method,
    endpoint: Endpoint,
    quota: QuotaClass,
    retry_of: Option<RetryCause>,
}

// A dispatched attempt, finished by the scheduler.
struct AttemptRecord {
    span: Span,
    dispatched_at: Instant,
    _in_flight: GaugeGuard,
}

impl DispatchToken {
    /// Mark the attempt as dispatched: from here on the broker may receive
    /// the request. Returns the attempt's span, a child of the operation's.
    pub(crate) fn dispatch(&mut self) -> Span {
        if let Some(g) = self.grant.take() {
            g.consume();
        }
        self.dispatched.store(true, Ordering::Release);
        let o = &self.obs;
        let span = tracing::debug_span!(
            parent: &o.parent,
            "manja.http.attempt",
            operation_id = o.operation_id,
            attempt = o.number,
            method = o.method.as_str(),
            endpoint = o.endpoint.as_str(),
            http_status = tracing::field::Empty,
            error_class = tracing::field::Empty,
            stage = tracing::field::Empty,
        );
        if let (true, Some(cause)) = (o.number > 1, o.retry_of) {
            o.obs
                .counter(Labels::http_retry(o.method, o.endpoint, cause), 1);
        }
        let in_flight = o
            .obs
            .gauge(Labels::quota(Instrument::HttpInFlight, o.quota));
        in_flight.set(1);
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(AttemptRecord {
            span: span.clone(),
            dispatched_at: Instant::now(),
            _in_flight: in_flight,
        });
        span
    }
}

// Finishes the attempt in `slot`, if it was dispatched; an attempt dropped
// before finishing (its operation was cancelled) is recorded as cancelled.
struct AttemptFinisher<'a> {
    obs: &'a Observability,
    method: Method,
    endpoint: Endpoint,
    slot: Arc<Mutex<Option<AttemptRecord>>>,
}

impl AttemptFinisher<'_> {
    fn finish(&self, result: HttpAttemptResult, stage: TransportStage) {
        let Some(record) = self.slot.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        let secs = record.dispatched_at.elapsed().as_secs_f64();
        let labels = |i| Labels::http_attempt(i, self.method, self.endpoint, result);
        self.obs.counter(labels(Instrument::HttpAttemptsTotal), 1);
        self.obs
            .histogram(labels(Instrument::HttpAttemptDuration), secs);
        if result == HttpAttemptResult::AuthRejected {
            self.obs
                .counter(Labels::auth_rejection(TransportLabel::Http), 1);
        }
        if result != HttpAttemptResult::Ok {
            record.span.record("error_class", result.as_str());
        }
        record.span.record("stage", stage.as_str());
    }
}

impl Drop for AttemptFinisher<'_> {
    fn drop(&mut self) {
        self.finish(HttpAttemptResult::Cancelled, TransportStage::Started);
    }
}

/// The normalized result of one dispatched attempt.
pub(crate) fn attempt_result(e: &HttpError) -> HttpAttemptResult {
    match e.kind() {
        HttpErrorKind::HttpStatus => HttpAttemptResult::HttpStatus,
        HttpErrorKind::Broker => HttpAttemptResult::BrokerError,
        HttpErrorKind::AuthRejected => HttpAttemptResult::AuthRejected,
        HttpErrorKind::Decode => HttpAttemptResult::DecodeError,
        HttpErrorKind::Cancelled => HttpAttemptResult::Cancelled,
        _ if e.is_timeout() => HttpAttemptResult::Timeout,
        _ => HttpAttemptResult::TransportError,
    }
}

// Measures one admission wait: its span, its waiter gauge contribution and
// its duration. Dropped unfinished, the wait was cancelled.
struct AdmissionWatch<'a> {
    obs: &'a Observability,
    quota: QuotaClass,
    span: Span,
    started: Instant,
    _waiter: GaugeGuard,
    waiting: Option<&'a AtomicUsize>,
    done: bool,
}

impl<'a> AdmissionWatch<'a> {
    fn start(
        obs: &'a Observability,
        quota: QuotaClass,
        parent: Option<&Span>,
        operation_id: Option<u64>,
        waiting: Option<&'a AtomicUsize>,
    ) -> Self {
        let span = tracing::debug_span!(
            parent: parent.and_then(Span::id),
            "manja.http.admission",
            operation_id = operation_id,
            quota_class = quota.as_str(),
            wait_ms = tracing::field::Empty,
            admission_result = tracing::field::Empty,
        );
        let waiter = obs.gauge(Labels::quota(Instrument::HttpAdmissionWaiters, quota));
        waiter.set(1);
        if let Some(w) = waiting {
            w.fetch_add(1, Ordering::Relaxed);
        }
        Self {
            obs,
            quota,
            span,
            started: Instant::now(),
            _waiter: waiter,
            waiting,
            done: false,
        }
    }

    fn finish(&mut self, result: AdmissionResult) {
        if std::mem::replace(&mut self.done, true) {
            return;
        }
        let waited = self.started.elapsed();
        self.obs.histogram(
            Labels::admission_wait(self.quota, result),
            waited.as_secs_f64(),
        );
        self.span
            .record("wait_ms", waited.as_millis().min(u64::MAX as u128) as u64);
        self.span.record("admission_result", result.as_str());
        if let Some(w) = self.waiting {
            w.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Drop for AdmissionWatch<'_> {
    fn drop(&mut self) {
        self.finish(AdmissionResult::Cancelled);
    }
}

// Wait for admission under an [`AdmissionWatch`].
#[allow(clippy::too_many_arguments)]
async fn acquire_observed(
    admission: &Admission,
    rate: RateClass,
    order_id: Option<&str>,
    wait: Duration,
    deadline_bound: bool,
    quota: QuotaClass,
    ctx: Option<&OpObs<'_>>,
    obs: &Observability,
) -> Result<AdmissionGrant, AdmissionError> {
    let mut watch = AdmissionWatch::start(
        obs,
        quota,
        ctx.map(|c| c.span),
        ctx.map(|c| c.operation_id),
        ctx.map(|c| c.waiting),
    );
    let span = watch.span.clone();
    let result = admission
        .acquire(rate, order_id, wait)
        .instrument(span)
        .await;
    watch.finish(match &result {
        Ok(_) => AdmissionResult::Granted,
        Err(AdmissionError::WaitExpired) if deadline_bound => AdmissionResult::Deadline,
        Err(_) => AdmissionResult::Rejected,
    });
    result
}

/// Context of one attempt.
pub(crate) struct AttemptCtx {
    pub(crate) number: u32,
    pub(crate) token: DispatchToken,
}

/// One logical operation.
pub(crate) struct OpSpec<'a> {
    pub(crate) method: Method,
    pub(crate) endpoint: Endpoint,
    pub(crate) order_id: Option<&'a str>,
}

/// The instrumentation context of one operation, passed explicitly through
/// every await point rather than held in a task-local.
pub(crate) struct OpObs<'a> {
    pub(crate) obs: &'a Observability,
    /// The operation span; admission and attempt spans are its children.
    pub(crate) span: &'a Span,
    pub(crate) operation_id: u64,
    /// The transport's count of its operations waiting in this quota class.
    pub(crate) waiting: &'a AtomicUsize,
}

// A small, seedable PRNG for jitter (SplitMix64); jitter needs spread, not
// cryptographic strength.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// The retry cause of a failed attempt, if its class may retry it.
pub(crate) fn retry_cause(e: &HttpError) -> Option<RetryCause> {
    match (e.kind(), e.http_status()) {
        (_, Some(429)) => Some(RetryCause::Http429),
        (_, Some(502..=504)) => Some(RetryCause::Http5xx),
        (HttpErrorKind::Transport, _) if e.is_timeout() => Some(RetryCause::Timeout),
        (HttpErrorKind::Transport, None) => Some(RetryCause::TransportError),
        _ => None,
    }
}

/// Runs operations under [`SchedulerLimits`].
pub(crate) struct Scheduler {
    limits: SchedulerLimits,
    rng: Mutex<SplitMix64>,
}

impl Scheduler {
    pub(crate) fn new(limits: SchedulerLimits) -> Self {
        let seed = limits.jitter_seed.unwrap_or_else(|| {
            use std::hash::{BuildHasher, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default(),
            );
            h.finish()
        });
        Self {
            limits,
            rng: Mutex::new(SplitMix64(seed)),
        }
    }

    pub(crate) fn limits(&self) -> &SchedulerLimits {
        &self.limits
    }

    /// Full-jitter backoff before retry number `retry` (1-based).
    fn backoff(&self, retry: u32) -> Duration {
        let (initial, cap) = (self.limits.backoff_initial, self.limits.backoff_cap);
        let exp = initial.saturating_mul(1u32 << (retry - 1).min(16)).min(cap);
        let r = self.rng.lock().unwrap_or_else(|e| e.into_inner()).next();
        let nanos = exp.as_nanos() as u64;
        Duration::from_nanos(if nanos == 0 { 0 } else { r % (nanos + 1) })
    }

    /// Admit capacity for `target` now, valid for the permit validity bound.
    pub(crate) async fn admit(
        &self,
        admission: &Admission,
        target: PermitTarget,
        obs: &Observability,
    ) -> Result<DispatchPermit, HttpError> {
        let (method, endpoint) = target.operation();
        let wait = admission.limits().wait();
        let quota = RetryClass::of(method, endpoint).quota_class();
        let rate = RateClass::of(method, endpoint);
        let grant = acquire_observed(
            admission,
            rate,
            target.order_id(),
            wait,
            false,
            quota,
            None,
            obs,
        )
        .await
        .map_err(|e| admission_error(e, method, endpoint, 0, false))?;
        Ok(DispatchPermit {
            grant,
            expires_at: Instant::now() + self.limits.permit_validity,
            target,
        })
    }

    /// Run `attempt` under the operation's deadline, admission and retry
    /// policy, optionally starting from an externally admitted permit.
    pub(crate) async fn run<T, F, Fut>(
        &self,
        spec: OpSpec<'_>,
        admission: &Admission,
        permit: Option<DispatchPermit>,
        ctx: &OpObs<'_>,
        mut attempt: F,
    ) -> Result<T, HttpError>
    where
        F: FnMut(AttemptCtx) -> Fut,
        Fut: std::future::Future<Output = Result<T, HttpError>>,
    {
        let (method, endpoint) = (spec.method, spec.endpoint);
        let class = RetryClass::of(method, endpoint);
        let rate = RateClass::of(method, endpoint);
        let quota = class.quota_class();
        let max_attempts = self.limits.max_attempts(class);
        let mut retry_of = None;
        let start = Instant::now();
        let deadline_at = start + self.limits.deadline(class);
        let mut permit = permit;
        let mut deadline_at_permit = None;
        if let Some(p) = &permit {
            check_permit(p, admission, method, endpoint, spec.order_id)?;
            deadline_at_permit = Some(p.expires_at);
        }
        let retry_info = |n: u32| RetryInfo {
            attempts: n,
            max_attempts,
        };
        for n in 1..=max_attempts {
            let now = Instant::now();
            if now >= deadline_at {
                return Err(deadline_error(method, endpoint, n).with_retry(retry_info(n - 1)));
            }
            let remaining = deadline_at - now;
            let grant = match permit.take() {
                Some(p) => p.grant,
                None => {
                    let wait = admission.limits().wait().min(remaining);
                    let deadline_bound = wait < admission.limits().wait();
                    acquire_observed(
                        admission,
                        rate,
                        spec.order_id,
                        wait,
                        deadline_bound,
                        quota,
                        Some(ctx),
                        ctx.obs,
                    )
                    .await
                    .map_err(|e| {
                        admission_error(e, method, endpoint, n, deadline_bound)
                            .with_retry(retry_info(n - 1))
                    })?
                }
            };
            let now = Instant::now();
            if now >= deadline_at || deadline_at_permit.is_some_and(|p| now >= p) {
                drop(grant);
                return Err(deadline_error(method, endpoint, n).with_retry(retry_info(n - 1)));
            }
            let attempt_timeout = self.limits.attempt_timeout.min(deadline_at - now);
            let dispatched = Arc::new(AtomicBool::new(false));
            let finisher = AttemptFinisher {
                obs: ctx.obs,
                method,
                endpoint,
                slot: Arc::new(Mutex::new(None)),
            };
            let attempt_ctx = AttemptCtx {
                number: n,
                token: DispatchToken {
                    grant: Some(grant),
                    dispatched: dispatched.clone(),
                    obs: AttemptObs {
                        obs: ctx.obs.clone(),
                        parent: ctx.span.clone(),
                        operation_id: ctx.operation_id,
                        number: n,
                        method,
                        endpoint,
                        quota,
                        retry_of,
                    },
                    slot: finisher.slot.clone(),
                },
            };
            let result = match tokio::time::timeout(attempt_timeout, attempt(attempt_ctx)).await {
                Ok(r) => r,
                Err(_) => {
                    let stage = if dispatched.load(Ordering::Acquire) {
                        TransportStage::Started
                    } else {
                        TransportStage::NotStarted
                    };
                    Err(
                        HttpError::new(HttpErrorKind::Transport, method, endpoint, stage)
                            .with_attempt(n)
                            .with_timeout()
                            .with_detail("the attempt timeout elapsed"),
                    )
                }
            };
            match &result {
                Ok(_) => finisher.finish(HttpAttemptResult::Ok, TransportStage::ResponseReceived),
                Err(e) => finisher.finish(attempt_result(e), e.stage()),
            }
            let err = match result {
                Ok(v) => return Ok(v),
                Err(e) => e.with_retry(retry_info(n)),
            };
            retry_of = retry_cause(&err);
            let retryable =
                matches!(class, RetryClass::Read | RetryClass::Calc) && retry_of.is_some();
            if !retryable || n == max_attempts {
                return Err(err);
            }
            let delay = self.backoff(n);
            if Instant::now() + delay >= deadline_at {
                return Err(err);
            }
            tokio::time::sleep(delay).await;
        }
        unreachable!("the loop returns on its last attempt")
    }
}

fn check_permit(
    p: &DispatchPermit,
    admission: &Admission,
    method: Method,
    endpoint: Endpoint,
    order_id: Option<&str>,
) -> Result<(), HttpError> {
    let refuse = |kind: HttpErrorKind, detail: &str| {
        HttpError::new(kind, method, endpoint, TransportStage::NotStarted).with_detail(detail)
    };
    if !p.belongs_to(admission) {
        return Err(refuse(
            HttpErrorKind::Admission,
            "the permit belongs to another admission scope",
        ));
    }
    if p.target.operation() != (method, endpoint) || p.target.order_id() != order_id {
        return Err(refuse(
            HttpErrorKind::Admission,
            "the permit was issued for a different operation",
        ));
    }
    if p.is_expired() {
        return Err(refuse(HttpErrorKind::Deadline, "the permit has expired"));
    }
    Ok(())
}

fn deadline_error(method: Method, endpoint: Endpoint, attempt: u32) -> HttpError {
    HttpError::new(
        HttpErrorKind::Deadline,
        method,
        endpoint,
        TransportStage::NotStarted,
    )
    .with_attempt(attempt)
    .with_timeout()
    .with_detail("the operation deadline expired before the attempt started")
}

fn admission_error(
    e: AdmissionError,
    method: Method,
    endpoint: Endpoint,
    attempt: u32,
    deadline_bound: bool,
) -> HttpError {
    let kind = if deadline_bound && e == AdmissionError::WaitExpired {
        HttpErrorKind::Deadline
    } else {
        HttpErrorKind::Admission
    };
    HttpError::new(kind, method, endpoint, TransportStage::NotStarted)
        .with_attempt(attempt)
        .with_detail(&e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kite::connect::admission::{AdmissionLimits, QuotaProfile};
    use std::sync::atomic::AtomicU32;

    impl Scheduler {
        // `run` and `admit` with a disabled observability scope.
        async fn run_q<T, F, Fut>(
            &self,
            spec: OpSpec<'_>,
            admission: &Admission,
            permit: Option<DispatchPermit>,
            attempt: F,
        ) -> Result<T, HttpError>
        where
            F: FnMut(AttemptCtx) -> Fut,
            Fut: std::future::Future<Output = Result<T, HttpError>>,
        {
            let obs = Observability::disabled();
            let span = Span::none();
            let waiting = AtomicUsize::new(0);
            let ctx = OpObs {
                obs: &obs,
                span: &span,
                operation_id: 0,
                waiting: &waiting,
            };
            self.run(spec, admission, permit, &ctx, attempt).await
        }

        async fn admit_q(
            &self,
            admission: &Admission,
            target: PermitTarget,
        ) -> Result<DispatchPermit, HttpError> {
            self.admit(admission, target, &Observability::disabled())
                .await
        }
    }

    fn scheduler() -> Scheduler {
        Scheduler::new(SchedulerLimits::default().with_jitter_seed(7))
    }

    fn admission() -> Admission {
        Admission::new(
            QuotaProfile::kite_v3(),
            AdmissionLimits::default()
                .with_wait(Duration::from_secs(60))
                .unwrap(),
        )
    }

    fn failing(status: u16, method: Method, endpoint: Endpoint) -> HttpError {
        HttpError::new(
            HttpErrorKind::HttpStatus,
            method,
            endpoint,
            TransportStage::ResponseReceived,
        )
        .with_status(status)
    }

    fn spec(method: Method, endpoint: Endpoint) -> OpSpec<'static> {
        OpSpec {
            method,
            endpoint,
            order_id: None,
        }
    }

    #[test]
    fn classes_follow_the_documented_policy() {
        use Endpoint as E;
        use Method as M;
        assert_eq!(RetryClass::of(M::Get, E::Orders), RetryClass::Read);
        assert_eq!(RetryClass::of(M::Post, E::MarginsBasket), RetryClass::Calc);
        assert_eq!(RetryClass::of(M::Post, E::ChargesOrders), RetryClass::Calc);
        assert_eq!(RetryClass::of(M::Post, E::OrdersVariety), RetryClass::Mut);
        assert_eq!(RetryClass::of(M::Put, E::OrdersVarietyId), RetryClass::Mut);
        assert_eq!(
            RetryClass::of(M::Delete, E::OrdersVarietyId),
            RetryClass::Mut
        );
        assert_eq!(RetryClass::of(M::Put, E::Positions), RetryClass::Mut);
        assert_eq!(RetryClass::of(M::Post, E::SessionToken), RetryClass::Sess);
        assert_eq!(RetryClass::of(M::Delete, E::SessionToken), RetryClass::Sess);
        // An undocumented non-GET endpoint is never retried.
        assert_eq!(RetryClass::of(M::Post, E::Unknown), RetryClass::Mut);
        let l = SchedulerLimits::default();
        assert_eq!(l.max_attempts(RetryClass::Mut), 1);
        assert_eq!(l.max_attempts(RetryClass::Sess), 1);
        assert_eq!(l.max_attempts(RetryClass::Read), 3);
    }

    #[test]
    fn bounds_are_validated() {
        let d = SchedulerLimits::default();
        assert!(d
            .clone()
            .with_operation_deadline(Duration::from_secs(1))
            .is_err()); // < attempt timeout
        assert!(d
            .clone()
            .with_attempt_timeout(Duration::from_millis(99))
            .is_err());
        assert!(d
            .clone()
            .with_attempt_timeout(Duration::from_secs(31))
            .is_err()); // > deadline
        assert!(d.clone().with_read_attempts(0).is_err());
        assert!(d.clone().with_read_attempts(6).is_err());
        assert!(d.clone().with_read_attempts(5).is_ok());
        assert!(d
            .clone()
            .with_backoff(Duration::from_secs(1), Duration::from_millis(500))
            .is_err());
        assert!(d
            .clone()
            .with_backoff(Duration::from_millis(10), Duration::from_millis(10))
            .is_ok());
        assert!(d
            .clone()
            .with_session_deadline(Duration::from_secs(61))
            .is_err());
        assert!(d
            .clone()
            .with_permit_validity(Duration::from_millis(9))
            .is_err());
        assert!(d.with_permit_validity(Duration::from_secs(30)).is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_429_on_a_read_terminates_within_attempt_and_time_budgets() {
        let s = scheduler();
        let a = admission();
        let calls = AtomicU32::new(0);
        let start = Instant::now();
        let err = s
            .run_q(spec(Method::Get, Endpoint::Orders), &a, None, |mut ctx| {
                calls.fetch_add(1, Ordering::Relaxed);
                ctx.token.dispatch();
                async { Err::<(), _>(failing(429, Method::Get, Endpoint::Orders)) }
            })
            .await
            .unwrap_err();
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        assert_eq!(err.http_status(), Some(429));
        assert_eq!(err.retry().unwrap().attempts, 3);
        // Two capped, jittered backoffs of at most 250 ms and 500 ms.
        assert!(start.elapsed() <= Duration::from_millis(750));
    }

    #[tokio::test(start_paused = true)]
    async fn mutations_and_session_operations_make_exactly_one_attempt() {
        let s = scheduler();
        let a = admission();
        for (m, e) in [
            (Method::Post, Endpoint::OrdersVariety),
            (Method::Put, Endpoint::OrdersVarietyId),
            (Method::Delete, Endpoint::OrdersVarietyId),
            (Method::Put, Endpoint::Positions),
            (Method::Post, Endpoint::SessionToken),
            (Method::Delete, Endpoint::SessionToken),
        ] {
            for status in [429, 502, 504] {
                let calls = AtomicU32::new(0);
                let err = s
                    .run_q(spec(m, e), &a, None, |mut ctx| {
                        calls.fetch_add(1, Ordering::Relaxed);
                        ctx.token.dispatch();
                        async move { Err::<(), _>(failing(status, m, e)) }
                    })
                    .await
                    .unwrap_err();
                assert_eq!(calls.load(Ordering::Relaxed), 1, "{m:?} {e:?} {status}");
                assert_eq!(err.retry().unwrap().max_attempts, 1);
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_total_deadline_covers_admission_attempts_and_backoff() {
        let limits = SchedulerLimits::default()
            .with_attempt_timeout(Duration::from_millis(400))
            .unwrap()
            .with_operation_deadline(Duration::from_secs(1))
            .unwrap()
            .with_read_attempts(5)
            .unwrap()
            .with_jitter_seed(1);
        let s = Scheduler::new(limits);
        let a = admission();
        let start = Instant::now();
        // Every attempt stalls until its timeout.
        let err = s
            .run_q(spec(Method::Get, Endpoint::Trades), &a, None, |mut ctx| {
                ctx.token.dispatch();
                std::future::pending::<Result<(), HttpError>>()
            })
            .await
            .unwrap_err();
        assert!(
            start.elapsed() <= Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
        assert!(err.is_timeout());
        assert!(matches!(
            err.kind(),
            HttpErrorKind::Transport | HttpErrorKind::Deadline
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn an_attempt_timeout_before_dispatch_reports_not_started() {
        let s = scheduler();
        let a = admission();
        let err = s
            .run_q(
                spec(Method::Post, Endpoint::OrdersVariety),
                &a,
                None,
                |_ctx| std::future::pending::<Result<(), HttpError>>(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.stage(), TransportStage::NotStarted);
        let err = s
            .run_q(
                spec(Method::Post, Endpoint::OrdersVariety),
                &a,
                None,
                |mut ctx| {
                    ctx.token.dispatch();
                    std::future::pending::<Result<(), HttpError>>()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.stage(), TransportStage::Started);
        assert!(err.may_have_reached_broker());
    }

    #[tokio::test(start_paused = true)]
    async fn permits_are_scoped_targeted_expiring_and_single_use() {
        let s = scheduler();
        let a = admission();
        let never = |_ctx: AttemptCtx| async { Ok::<(), HttpError>(()) };
        let calls = AtomicU32::new(0);
        let counted = |mut ctx: AttemptCtx| {
            calls.fetch_add(1, Ordering::Relaxed);
            ctx.token.dispatch();
            async { Ok::<(), HttpError>(()) }
        };
        // Wrong scope.
        let other = admission();
        let p = s.admit_q(&other, PermitTarget::PlaceOrder).await.unwrap();
        let e = s
            .run_q(
                spec(Method::Post, Endpoint::OrdersVariety),
                &a,
                Some(p),
                never,
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind(), e.stage()),
            (HttpErrorKind::Admission, TransportStage::NotStarted)
        );
        // Wrong target.
        let p = s.admit_q(&a, PermitTarget::CancelOrder).await.unwrap();
        let e = s
            .run_q(
                spec(Method::Post, Endpoint::OrdersVariety),
                &a,
                Some(p),
                never,
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind(), HttpErrorKind::Admission);
        // Expired.
        let p = s.admit_q(&a, PermitTarget::PlaceOrder).await.unwrap();
        tokio::time::advance(Duration::from_secs(2)).await;
        let e = s
            .run_q(
                spec(Method::Post, Endpoint::OrdersVariety),
                &a,
                Some(p),
                never,
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind(), HttpErrorKind::Deadline);
        // Valid: dispatched exactly once, without re-admission.
        let waiters_before = a.waiters();
        let p = s.admit_q(&a, PermitTarget::PlaceOrder).await.unwrap();
        s.run_q(
            spec(Method::Post, Endpoint::OrdersVariety),
            &a,
            Some(p),
            counted,
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(a.waiters(), waiters_before);
    }

    #[tokio::test(start_paused = true)]
    async fn modify_permits_are_bound_to_their_order() {
        let s = scheduler();
        let a = admission();
        let p = s
            .admit_q(
                &a,
                PermitTarget::ModifyOrder {
                    order_id: "A".into(),
                },
            )
            .await
            .unwrap();
        let e = s
            .run_q(
                OpSpec {
                    method: Method::Put,
                    endpoint: Endpoint::OrdersVarietyId,
                    order_id: Some("B"),
                },
                &a,
                Some(p),
                |_ctx| async { Ok::<(), HttpError>(()) },
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind(), HttpErrorKind::Admission);
    }

    #[test]
    fn jitter_is_capped_and_reproducible() {
        let limits = SchedulerLimits::default().with_jitter_seed(42);
        let a = Scheduler::new(limits.clone());
        let b = Scheduler::new(limits);
        for retry in 1..20 {
            let d = a.backoff(retry);
            assert!(d <= Duration::from_secs(5));
            assert_eq!(d, b.backoff(retry));
        }
    }
}
