//! Error types.
//!
//! [`ManjaError`] is the crate's top-level error. Every failure of an
//! HTTP operation is its [`ManjaError::Http`] variant, carrying an
//! [`HttpError`]: a stable, inspectable category ([`HttpErrorKind`]) plus
//! metadata — method, endpoint template, HTTP status if one was received,
//! the broker's `error_type` and bounded message if any, attempt number,
//! [`TransportStage`], timeout and cancellation context, and retry
//! metadata. The contract is the category and the accessors, not the
//! `Display` text.
//!
//! An `HttpError` never contains a request URL, a request or response body,
//! a header, a credential or a checksum. Free text from the broker is
//! bounded and passed through
//! [`BoundedText::sanitize`](crate::kite::obs::diagnostics::BoundedText::sanitize)
//! before it is kept. A detail describing a response that failed to parse
//! or decode names what failed and where, never a value from the body.
//!
//! Stage evidence is deliberately conservative. [`TransportStage::NotStarted`]
//! is reported only when the SDK has affirmative local evidence that the
//! request never left the process, for example a failure to build it or to
//! connect. After that point a timeout, a lost response or a malformed reply
//! says nothing about whether the broker acted on the request.
//!
use std::fmt;

use crate::kite::connect::credentials::CredentialError;
use crate::kite::connect::models::IngressStamp;
use crate::kite::envelope::SourceIdentity;
use crate::kite::obs::diagnostics::BoundedText;
use crate::kite::obs::schema::{AdmissionCause, Endpoint, Method};
use crate::kite::protocol::Inbound;
use crate::kite::protocol::enums::wire_enum;

/// A `Result` alias where the `Err` case is `manja::kite::ManjaError`.
pub type Result<T> = std::result::Result<T, ManjaError>;

/// All errors that may occur when using the `manja` crate.
///
/// Non-exhaustive: new variants may be added.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManjaError {
    /// A failed HTTP operation, with inspectable stage evidence.
    #[error("{0}")]
    Http(HttpError),

    /// Invalid credential material supplied to a constructor.
    #[error("invalid credentials: {0}")]
    Credential(#[from] CredentialError),
}

impl From<HttpError> for ManjaError {
    fn from(e: HttpError) -> Self {
        ManjaError::Http(e)
    }
}

impl ManjaError {
    /// The HTTP error, if this is one.
    pub fn as_http(&self) -> Option<&HttpError> {
        match self {
            Self::Http(e) => Some(e),
            _ => None,
        }
    }
}

/// Category of an [`HttpError`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HttpErrorKind {
    /// Local validation of the request failed before admission; nothing was
    /// sent.
    Validation,
    /// The client or request could not be configured or built; nothing was
    /// sent.
    Configuration,
    /// Admission capacity was unavailable before transport started.
    Admission,
    /// The operation deadline expired.
    Deadline,
    /// The transport failed or timed out.
    Transport,
    /// A non-success HTTP status without a broker error envelope.
    HttpStatus,
    /// The broker returned an error envelope.
    Broker,
    /// The broker rejected the credentials (`TokenException` or HTTP 403,
    /// `kite:exceptions.md:20,35`).
    AuthRejected,
    /// A response was received but could not be decoded, was oversized, or
    /// lacked the required success envelope or payload.
    Decode,
    /// The operation's future was dropped before it finished. Recorded in
    /// `HTTPClient::diagnostics`; a call never returns it.
    Cancelled,
}

/// How far an HTTP attempt got, as far as the SDK can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransportStage {
    /// Affirmative local evidence that no request left the process.
    NotStarted,
    /// The request may have reached the broker; no response was received.
    Started,
    /// A response status was received.
    ResponseReceived,
    /// The stage cannot be determined.
    Unknown,
}

impl TransportStage {
    /// Stable name, as used in span fields.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Started => "started",
            Self::ResponseReceived => "response_received",
            Self::Unknown => "unknown",
        }
    }
}

/// The broker's error envelope, when one was received.
#[derive(Clone, Debug, PartialEq)]
pub struct BrokerError {
    error_type: Option<Inbound<KiteApiException>>,
    message: Option<BoundedText>,
}

#[cfg_attr(not(feature = "http"), allow(dead_code))]
impl BrokerError {
    /// A broker error as an envelope would carry it: `error_type` is kept
    /// as a known or unknown value and `message` is bounded and sanitized.
    pub fn new(error_type: Option<&str>, message: Option<&str>) -> Self {
        Self {
            error_type: error_type.map(Inbound::from_wire),
            message: message.map(|m| {
                BoundedText::sanitize(m, crate::kite::obs::diagnostics::DEFAULT_TEXT_BYTES)
            }),
        }
    }

    /// The `error_type`: known, unknown (preserved), or `None` when absent.
    pub fn error_type(&self) -> Option<&Inbound<KiteApiException>> {
        self.error_type.as_ref()
    }

    /// The broker's message, bounded and sanitized.
    pub fn message(&self) -> Option<&BoundedText> {
        self.message.as_ref()
    }
}

/// Retry metadata of an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryInfo {
    /// Attempts actually made, including the failed one reported.
    pub attempts: u32,
    /// Attempts the policy allowed.
    pub max_attempts: u32,
}

/// A failed HTTP operation.
///
/// Two questions answer most of what to do next:
///
/// - **What happened?** [`kind`](Self::kind) is the category: an invalid
///   request, a rate limit, a deadline, a network failure, an error from
///   Kite, rejected credentials or an unreadable response.
///   [`broker`](Self::broker) holds Kite's own error type and message, when
///   Kite sent one.
/// - **Did Kite see it?** [`stage`](Self::stage) says how far the request
///   got, and [`may_have_reached_broker`](Self::may_have_reached_broker) is
///   `false` only when manja knows the request never left the process. After
///   a lost response to an order, check the order book before sending the
///   order again.
///
/// It also reports the method, the endpoint template, the HTTP status, the
/// attempt number and retry metadata, whether a timeout ended it
/// ([`is_timeout`](Self::is_timeout)), a bounded detail and, for transport
/// failures, the underlying error through `source()`. It never holds a URL,
/// a body, a header or a credential. It is pointer-sized, because the
/// details are boxed.
///
/// ```
/// use manja::kite::error::{HttpError, HttpErrorKind};
///
/// fn advice(e: &HttpError) -> String {
///     if e.kind() == HttpErrorKind::AuthRejected {
///         return "the access token is no longer valid: log in again".into();
///     }
///     if e.may_have_reached_broker() {
///         format!("{e}: Kite may have acted on it, so check before repeating a change")
///     } else {
///         format!("{e}: nothing reached Kite")
///     }
/// }
/// ```
#[derive(Debug)]
pub struct HttpError(Box<HttpErrorInner>);

#[derive(Debug)]
struct HttpErrorInner {
    kind: HttpErrorKind,
    method: Method,
    endpoint: Endpoint,
    http_status: Option<u16>,
    broker: Option<BrokerError>,
    attempt: u32,
    stage: TransportStage,
    timed_out: bool,
    retry: Option<RetryInfo>,
    detail: Option<BoundedText>,
    admission_cause: Option<AdmissionCause>,
    retry_after: Option<std::time::Duration>,
    resets_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    identity: Option<SourceIdentity>,
    operation_id: Option<u64>,
    sent_at: Option<IngressStamp>,
    received_at: Option<IngressStamp>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

#[cfg_attr(not(feature = "http"), allow(dead_code))]
impl HttpError {
    /// An error of `kind` for `method` on `endpoint` that got as far as
    /// `stage`, with nothing else set: attempt 0, no status, no broker
    /// error, no detail, no stamps. The `with_*` builders add the rest.
    /// Public so that a broker double or a fixture test can build the
    /// error the SDK would return; the SDK itself never puts a value from
    /// a response body in an error, and a builder of one should not either.
    pub fn new(
        kind: HttpErrorKind,
        method: Method,
        endpoint: Endpoint,
        stage: TransportStage,
    ) -> Self {
        Self(Box::new(HttpErrorInner {
            kind,
            method,
            endpoint,
            http_status: None,
            broker: None,
            attempt: 0,
            stage,
            timed_out: false,
            retry: None,
            detail: None,
            admission_cause: None,
            retry_after: None,
            resets_at: None,
            identity: None,
            operation_id: None,
            sent_at: None,
            received_at: None,
            source: None,
        }))
    }

    /// The operation this error belongs to: the client's identity and the
    /// operation ID. Set once per operation, on the way out; kept if
    /// already set.
    pub fn with_operation(mut self, identity: SourceIdentity, operation_id: u64) -> Self {
        if self.0.identity.is_none() {
            self.0.identity = Some(identity);
        }
        if self.0.operation_id.is_none() {
            self.0.operation_id = Some(operation_id);
        }
        self
    }

    /// When the failed attempt was dispatched; kept if already set.
    pub fn with_sent_at(mut self, sent_at: IngressStamp) -> Self {
        if self.0.sent_at.is_none() {
            self.0.sent_at = Some(sent_at);
        }
        self
    }

    /// When the failed attempt's response head arrived; kept if already set.
    pub fn with_received_at(mut self, received_at: IngressStamp) -> Self {
        if self.0.received_at.is_none() {
            self.0.received_at = Some(received_at);
        }
        self
    }

    /// The admission facts of an `Admission` refusal: the cause and, when
    /// known, the time until the next slot or the instant the ceiling resets.
    pub fn with_admission(
        mut self,
        cause: AdmissionCause,
        retry_after: Option<std::time::Duration>,
        resets_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    ) -> Self {
        self.0.admission_cause = Some(cause);
        self.0.retry_after = retry_after;
        self.0.resets_at = resets_at;
        self
    }

    /// The HTTP status of the response, when one was received.
    pub fn with_status(mut self, status: u16) -> Self {
        self.0.http_status = Some(status);
        self
    }

    /// The broker's error envelope, when one was received.
    pub fn with_broker(mut self, broker: BrokerError) -> Self {
        self.0.broker = Some(broker);
        self
    }

    /// The number of the attempt that failed: 1 for a first try.
    pub fn with_attempt(mut self, attempt: u32) -> Self {
        self.0.attempt = attempt;
        self
    }

    /// Mark the error as ended by a timeout (`is_timeout`).
    pub fn with_timeout(mut self) -> Self {
        self.0.timed_out = true;
        self
    }

    /// The retry metadata of the operation.
    pub fn with_retry(mut self, retry: RetryInfo) -> Self {
        self.0.retry = Some(retry);
        self
    }

    /// A bounded, sanitized detail for people. The SDK never puts a value
    /// taken from a response body here; a builder of errors should not
    /// either.
    pub fn with_detail(mut self, detail: &str) -> Self {
        self.0.detail = Some(BoundedText::sanitize(
            detail,
            crate::kite::obs::diagnostics::DEFAULT_TEXT_BYTES,
        ));
        self
    }

    /// The underlying error, reported through `std::error::Error::source`.
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.0.source = Some(Box::new(source));
        self
    }

    /// The category.
    pub fn kind(&self) -> HttpErrorKind {
        self.0.kind
    }

    /// HTTP method.
    pub fn method(&self) -> Method {
        self.0.method
    }

    /// Endpoint template, never a concrete path or URL.
    pub fn endpoint(&self) -> Endpoint {
        self.0.endpoint
    }

    /// HTTP status, if a response status was received.
    pub fn http_status(&self) -> Option<u16> {
        self.0.http_status
    }

    /// The broker error envelope, if one was received.
    pub fn broker(&self) -> Option<&BrokerError> {
        self.0.broker.as_ref()
    }

    /// Attempt number (1-based) of the reported failure; 0 when no attempt
    /// started.
    pub fn attempt(&self) -> u32 {
        self.0.attempt
    }

    /// How far the attempt got.
    pub fn stage(&self) -> TransportStage {
        self.0.stage
    }

    /// Whether an attempt or operation timeout caused the failure.
    pub fn is_timeout(&self) -> bool {
        self.0.timed_out
    }

    /// Whether the kind is [`HttpErrorKind::Cancelled`]. A call never returns
    /// such an error: a dropped operation is recorded in
    /// `HTTPClient::diagnostics` instead.
    pub fn is_cancelled(&self) -> bool {
        self.0.kind == HttpErrorKind::Cancelled
    }

    /// Retry metadata, when a retry policy applied.
    pub fn retry(&self) -> Option<RetryInfo> {
        self.0.retry
    }

    /// Why admission did not grant capacity, for an [`HttpErrorKind::Admission`]
    /// error (and a `Deadline` error that expired while a read waited for
    /// admission); `None` for every other kind.
    pub fn admission_cause(&self) -> Option<AdmissionCause> {
        self.0.admission_cause
    }

    /// For a mutation or session operation refused because a window was
    /// full: the time until the lane's next slot, as of the refusal. The
    /// SDK does not wait it out; the caller decides.
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        self.0.retry_after
    }

    /// For a placement refused by the daily ceiling: when the IST day turns
    /// and the ceiling resets. There is no earlier recovery.
    pub fn resets_at(&self) -> Option<chrono::DateTime<chrono::FixedOffset>> {
        self.0.resets_at
    }

    /// The client's source identity (producer, run and feed), on every
    /// error an operation produces; the same value its responses carry.
    pub fn source(&self) -> Option<&SourceIdentity> {
        self.0.identity.as_ref()
    }

    /// The operation's ID, the `operation_id` its span records, on every
    /// error an operation produces.
    pub fn operation_id(&self) -> Option<u64> {
        self.0.operation_id
    }

    /// When the failed attempt was dispatched: `Some` once the request left
    /// the process (stage `Started` or later), `None` for an error with
    /// stage `NotStarted`.
    pub fn sent_at(&self) -> Option<IngressStamp> {
        self.0.sent_at
    }

    /// When the failed attempt's response head arrived: `Some` only with
    /// stage `ResponseReceived`.
    pub fn received_at(&self) -> Option<IngressStamp> {
        self.0.received_at
    }

    /// A bounded, sanitized description, such as why a body failed to
    /// decode. Never a body dump.
    pub fn detail(&self) -> Option<&BoundedText> {
        self.0.detail.as_ref()
    }

    /// Whether the broker refused a sell order because the holdings need
    /// authorisation at the depository: HTTP 428 (`kite:portfolio.md:512`).
    /// Start the flow with `Portfolio::authorise_holdings`, then retry the
    /// order once the user has finished it.
    pub fn requires_holdings_authorisation(&self) -> bool {
        self.0.http_status == Some(428)
    }

    /// Whether the request may have reached the broker. `true` unless the
    /// SDK has affirmative evidence that it did not.
    pub fn may_have_reached_broker(&self) -> bool {
        self.0.stage != TransportStage::NotStarted
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} error on {} {} (stage {}",
            self.0.kind,
            self.0.method.as_str(),
            self.0.endpoint.as_str(),
            self.0.stage.as_str()
        )?;
        if let Some(status) = self.0.http_status {
            write!(f, ", HTTP {status}")?;
        }
        if self.0.attempt > 0 {
            write!(f, ", attempt {}", self.0.attempt)?;
        }
        f.write_str(")")?;
        if let Some(b) = &self.0.broker {
            if let Some(t) = &b.error_type {
                write!(f, ": {t}")?;
            }
            if let Some(m) = &b.message {
                write!(f, ": {m}")?;
            }
        } else if let Some(d) = &self.0.detail {
            write!(f, ": {d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for HttpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0
            .source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

/// A detail for a JSON parse or decode failure: `context`, then the error's
/// category and, when known, its position. serde's message is left out
/// because it can quote the input (`invalid type: string "..."`, `unknown
/// variant ...`).
#[cfg(any(feature = "http", feature = "decoder"))]
pub(crate) fn json_error_detail(context: &str, e: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let category = match e.classify() {
        Category::Io => "read error",
        Category::Syntax => "syntax error",
        Category::Data => "data error",
        Category::Eof => "unexpected end of input",
    };
    // A value decoded from a `Value` has no position (line 0).
    if e.line() == 0 {
        format!("{context}: {category}")
    } else {
        format!(
            "{context}: {category} at line {} column {}",
            e.line(),
            e.column()
        )
    }
}

/// The documented broker exception types
/// (`kite:exceptions.md:18-28`).
///
/// An undocumented `error_type` is preserved as
/// [`Inbound::Unknown`] rather than
/// mapped to one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KiteApiException {
    /// Preceded by a 403 header, this indicates the expiry or invalidation of
    /// an authenticated session. This can be caused by the user logging out,
    /// a natural expiry, or the user logging into another Kite instance.
    /// The caller decides whether and how to log in again.
    TokenException,
    /// Represents user account related errors.
    UserException,
    /// Represents order related errors such as placement failures or a corrupt
    /// fetch.
    OrderException,
    /// Represents missing required fields or bad values for parameters.
    InputException,
    /// Represents insufficient funds required for order placement.
    MarginException,
    /// Represents insufficient holdings available to place a sell order for a
    /// specified instrument.
    HoldingException,
    /// Represents a network error where the API was unable to communicate
    /// with the Order Management System (OMS).
    NetworkException,
    /// Represents an internal system error where the API was unable to
    /// understand the response from the OMS to respond to a request.
    DataException,
    /// Represents an unclassified error. This should only happen rarely.
    GeneralException,
}

wire_enum!(KiteApiException {
    TokenException => "TokenException",
    UserException => "UserException",
    OrderException => "OrderException",
    InputException => "InputException",
    MarginException => "MarginException",
    HoldingException => "HoldingException",
    NetworkException => "NetworkException",
    DataException => "DataException",
    GeneralException => "GeneralException",
});
