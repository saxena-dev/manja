//! Error types.
//!
//! [`ManjaError`] is the crate's legacy top-level error. Every failure of an
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
use crate::kite::obs::diagnostics::BoundedText;
use crate::kite::obs::schema::{Endpoint, Method};
use crate::kite::protocol::enums::wire_enum;
use crate::kite::protocol::Inbound;

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

    /// Represents errors that occur during JSON deserialization.
    #[error("JSON deserialization error: {0}")]
    JSONDeserialize(#[from] serde_json::Error),

    /// Represents general I/O errors.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Represents internal errors within the `manja` crate.
    #[error("Internal `manja` error: {0}")]
    Internal(String),
}

impl From<&str> for ManjaError {
    fn from(value: &str) -> Self {
        ManjaError::Internal(value.to_string())
    }
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
    /// The operation was cancelled.
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
    pub(crate) fn new(error_type: Option<&str>, message: Option<&str>) -> Self {
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

/// A failed HTTP operation. Pointer-sized: the details are boxed.
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
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

#[cfg_attr(not(feature = "http"), allow(dead_code))]
impl HttpError {
    pub(crate) fn new(
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
            source: None,
        }))
    }

    pub(crate) fn with_status(mut self, status: u16) -> Self {
        self.0.http_status = Some(status);
        self
    }

    pub(crate) fn with_broker(mut self, broker: BrokerError) -> Self {
        self.0.broker = Some(broker);
        self
    }

    pub(crate) fn with_attempt(mut self, attempt: u32) -> Self {
        self.0.attempt = attempt;
        self
    }

    pub(crate) fn with_timeout(mut self) -> Self {
        self.0.timed_out = true;
        self
    }

    pub(crate) fn with_retry(mut self, retry: RetryInfo) -> Self {
        self.0.retry = Some(retry);
        self
    }

    pub(crate) fn with_detail(mut self, detail: &str) -> Self {
        self.0.detail = Some(BoundedText::sanitize(
            detail,
            crate::kite::obs::diagnostics::DEFAULT_TEXT_BYTES,
        ));
        self
    }

    pub(crate) fn with_source(
        mut self,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
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

    /// Whether the operation was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.0.kind == HttpErrorKind::Cancelled
    }

    /// Retry metadata, when a retry policy applied.
    pub fn retry(&self) -> Option<RetryInfo> {
        self.0.retry
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
