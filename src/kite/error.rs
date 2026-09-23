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
//! before it is kept.
//!
//! Stage evidence is deliberately conservative. [`TransportStage::NotStarted`]
//! is reported only when the SDK has affirmative local evidence that the
//! request never left the process, for example a failure to build it or to
//! connect. After that point a timeout, a lost response or a malformed reply
//! says nothing about whether the broker acted on the request.
//!
use std::env::VarError;
use std::fmt;

use fantoccini::error::CmdError;
use fantoccini::error::NewSessionError;
use reqwest::header::InvalidHeaderValue;

use crate::kite::connect::credentials::CredentialError;
use crate::kite::obs::diagnostics::BoundedText;
use crate::kite::obs::schema::{Endpoint, Method};
use crate::kite::protocol::enums::wire_enum;
use crate::kite::protocol::Inbound;

/// A `Result` alias where the `Err` case is `manja::kite::ManjaError`.
pub type Result<T> = std::result::Result<T, ManjaError>;

/// All errors that may occur when using the `manja` crate.
///
/// Non-exhaustive: new variants may be added. The WebDriver, TOTP,
/// environment and `reqwest` variants belong to the legacy browser-login
/// path and are scheduled for removal with it.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManjaError {
    /// A failed HTTP operation, with inspectable stage evidence.
    #[error("{0}")]
    Http(Box<HttpError>),

    /// Invalid credential material supplied to a constructor.
    #[error("invalid credentials: {0}")]
    Credential(#[from] CredentialError),

    /// Represents errors related to missing or invalid environment variables.
    #[error("Environment variable error: {0}")]
    EnvVarError(#[from] VarError),

    /// Represents errors related to invalid HTTP headers.
    #[error("Invalid header value: {0}")]
    InvalidHeaderValueError(#[from] InvalidHeaderValue),

    /// Represents errors related to starting a new WebDriver session.
    #[error("WebDriver new session error: {0}")]
    WebDriverNewSessionError(#[from] NewSessionError),

    /// Represents general WebDriver errors.
    #[error("WebDriver error: {0}")]
    WebDriverError(#[from] CmdError),

    /// Represents errors that occur during JSON deserialization.
    #[error("JSON deserialization error: {0}")]
    JSONDeserialize(#[from] serde_json::Error),

    /// Represents general I/O errors.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Represents HTTP client errors raised outside an HTTP operation, such
    /// as by the legacy login flow.
    #[error("HTTP error: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// Represents errors related to Time-based One-Time Password (TOTP) generation or validation.
    #[error("TOTP error: {0}")]
    TotpError(String),

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
        ManjaError::Http(Box::new(e))
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
    /// `kite-api-docs/docs/connect/v3/exceptions.md:20,35`).
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

/// A failed HTTP operation.
#[derive(Debug)]
#[non_exhaustive]
pub struct HttpError {
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

impl HttpError {
    pub(crate) fn new(
        kind: HttpErrorKind,
        method: Method,
        endpoint: Endpoint,
        stage: TransportStage,
    ) -> Self {
        Self {
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
        }
    }

    pub(crate) fn with_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    pub(crate) fn with_broker(mut self, broker: BrokerError) -> Self {
        self.broker = Some(broker);
        self
    }

    pub(crate) fn with_attempt(mut self, attempt: u32) -> Self {
        self.attempt = attempt;
        self
    }

    pub(crate) fn with_timeout(mut self) -> Self {
        self.timed_out = true;
        self
    }

    pub(crate) fn with_retry(mut self, retry: RetryInfo) -> Self {
        self.retry = Some(retry);
        self
    }

    pub(crate) fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(BoundedText::sanitize(
            detail,
            crate::kite::obs::diagnostics::DEFAULT_TEXT_BYTES,
        ));
        self
    }

    pub(crate) fn with_source(
        mut self,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// The category.
    pub fn kind(&self) -> HttpErrorKind {
        self.kind
    }

    /// HTTP method.
    pub fn method(&self) -> Method {
        self.method
    }

    /// Endpoint template, never a concrete path or URL.
    pub fn endpoint(&self) -> Endpoint {
        self.endpoint
    }

    /// HTTP status, if a response status was received.
    pub fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    /// The broker error envelope, if one was received.
    pub fn broker(&self) -> Option<&BrokerError> {
        self.broker.as_ref()
    }

    /// Attempt number (1-based) of the reported failure; 0 when no attempt
    /// started.
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// How far the attempt got.
    pub fn stage(&self) -> TransportStage {
        self.stage
    }

    /// Whether an attempt or operation timeout caused the failure.
    pub fn is_timeout(&self) -> bool {
        self.timed_out
    }

    /// Whether the operation was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.kind == HttpErrorKind::Cancelled
    }

    /// Retry metadata, when a retry policy applied.
    pub fn retry(&self) -> Option<RetryInfo> {
        self.retry
    }

    /// A bounded, sanitized description, such as why a body failed to
    /// decode. Never a body dump.
    pub fn detail(&self) -> Option<&BoundedText> {
        self.detail.as_ref()
    }

    /// Whether the request may have reached the broker. `true` unless the
    /// SDK has affirmative evidence that it did not.
    pub fn may_have_reached_broker(&self) -> bool {
        self.stage != TransportStage::NotStarted
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} error on {} {} (stage {}",
            self.kind,
            self.method.as_str(),
            self.endpoint.as_str(),
            self.stage.as_str()
        )?;
        if let Some(status) = self.http_status {
            write!(f, ", HTTP {status}")?;
        }
        if self.attempt > 0 {
            write!(f, ", attempt {}", self.attempt)?;
        }
        f.write_str(")")?;
        if let Some(b) = &self.broker {
            if let Some(t) = &b.error_type {
                write!(f, ": {t}")?;
            }
            if let Some(m) = &b.message {
                write!(f, ": {m}")?;
            }
        } else if let Some(d) = &self.detail {
            write!(f, ": {d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for HttpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

/// The documented broker exception types
/// (`kite-api-docs/docs/connect/v3/exceptions.md:18-28`).
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
