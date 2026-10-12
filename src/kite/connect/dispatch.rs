//! Dispatch: a mutation's attempt runs to completion whatever the caller
//! does with the future, and its outcome is never lost.
//!
//! Every mutation method returns a [`Dispatched`]. The call itself is
//! synchronous: it validates the request, renders its wire form, takes an
//! in-flight slot (`B-HTTP-10`), admits it now or refuses it (never waiting),
//! and, if admitted, spawns the attempt on the current Tokio runtime as a
//! task the client owns. From then on the attempt runs to completion. Its
//! outcome is delivered at most once to the `Dispatched` future, if it is
//! still alive, and exactly once to the client's [`OutcomeStream`] when one
//! was registered at build time (`B-HTTP-13`). Without a stream, an outcome
//! whose future was dropped before delivery is kept in a bounded ring of
//! [`ORPHANED_OUTCOMES`] in the client's diagnostics (`B-HTTP-14`) and
//! counted.
//!
//! A refused dispatch resolves at once with its refusal and produces no
//! outcome record: the refusal in the caller's hands is the record.
//! Dropping a `Dispatched` cancels nothing and loses nothing; once the
//! bytes left the process there is nothing to cancel, and the outcome
//! reaches the stream or the ring.
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Weak;
use std::task::{Context, Poll};

use tokio::sync::oneshot;

use crate::kite::connect::models::{IngressStamp, Response};
use crate::kite::envelope::SourceIdentity;
use crate::kite::error::{HttpError, HttpErrorKind, TransportStage};
use crate::kite::obs::schema::{Endpoint, HttpOperationResult, Method};
use crate::kite::protocol::OrderId;

/// Orphaned outcomes kept in diagnostics when no stream is registered
/// (`B-HTTP-14`, fixed).
pub const ORPHANED_OUTCOMES: usize = 16;

/// The longest correlation token, in bytes (`docs/contract.md` §2.17).
pub const MAX_CORRELATION_BYTES: usize = 64;

/// A mutation in flight, or refused: a future of its result.
///
/// Before it is awaited it tells the operation ID, the refusal if the call
/// was refused before transport, the SHA-256 of the body sent (`wire()`)
/// and the correlation token the handle carried. Awaiting a refused
/// dispatch resolves at once with the refusal. Dropping it detaches
/// nothing, because nothing was attached: the attempt completes and its
/// outcome reaches the client's outcome stream or orphan ring.
///
/// ```no_run
/// # use manja::kite::connect::client::HTTPClient;
/// # use manja::kite::connect::models::PlaceOrderRequest;
/// # async fn run(client: HTTPClient, request: PlaceOrderRequest) -> Result<(), Box<dyn std::error::Error>> {
/// let dispatched = client.orders().place_order(&request);
/// if let Some(refusal) = dispatched.refusal() {
///     println!("not sent: {refusal}");
/// }
/// let hash = dispatched.wire_sha256();
/// let receipt = dispatched.await?;
/// println!("{:?} accepted as {}", hash, receipt.data.order_id);
/// # Ok(()) }
/// ```
#[must_use = "a dispatched mutation is a future of its result; await it or keep its operation ID"]
pub struct Dispatched<T> {
    operation_id: u64,
    correlation: Option<String>,
    wire_sha256: Option<[u8; 32]>,
    method: Method,
    endpoint: Endpoint,
    state: DispatchState<T>,
    /// Where an outcome delivered to this future, but never taken from it,
    /// goes when the future is dropped.
    sink: Option<Weak<dyn OrphanSink>>,
}

enum DispatchState<T> {
    Refused(Option<HttpError>),
    InFlight(oneshot::Receiver<Delivery<T>>),
    Done,
}

/// What the attempt task hands the future: the result for the caller and
/// the outcome record, so that a future dropped with its result still
/// inside can hand the record to the orphan ring.
pub(crate) struct Delivery<T> {
    pub(crate) result: Result<Response<T>, HttpError>,
    pub(crate) outcome: Outcome,
}

/// The transport side of the orphan ring: takes an outcome that was
/// delivered to a `Dispatched` future which was then dropped unpolled.
pub(crate) trait OrphanSink: Send + Sync {
    fn orphaned_after_delivery(&self, outcome: Outcome);
}

impl<T> Drop for Dispatched<T> {
    fn drop(&mut self) {
        // Dropped before the attempt delivered: the task finds the receiver
        // closed and treats the outcome as orphaned itself. Dropped after:
        // the outcome sits here, and goes to the ring the same way. The
        // close comes first so that a send racing this drop either lands
        // before it (received here) or fails (orphaned by the task).
        if let DispatchState::InFlight(receiver) = &mut self.state
            && let Ok(delivery) = {
                receiver.close();
                receiver.try_recv()
            }
            && let Some(sink) = self.sink.as_ref().and_then(Weak::upgrade)
        {
            sink.orphaned_after_delivery(delivery.outcome);
        }
    }
}

impl<T> Dispatched<T> {
    pub(crate) fn refused(
        operation_id: u64,
        correlation: Option<String>,
        wire_sha256: Option<[u8; 32]>,
        method: Method,
        endpoint: Endpoint,
        error: HttpError,
    ) -> Self {
        Self {
            operation_id,
            correlation,
            wire_sha256,
            method,
            endpoint,
            state: DispatchState::Refused(Some(error)),
            sink: None,
        }
    }

    pub(crate) fn in_flight(
        operation_id: u64,
        correlation: Option<String>,
        wire_sha256: [u8; 32],
        method: Method,
        endpoint: Endpoint,
        receiver: oneshot::Receiver<Delivery<T>>,
        sink: Weak<dyn OrphanSink>,
    ) -> Self {
        Self {
            operation_id,
            correlation,
            wire_sha256: Some(wire_sha256),
            method,
            endpoint,
            state: DispatchState::InFlight(receiver),
            sink: Some(sink),
        }
    }

    /// The operation's ID: the `operation_id` of its `manja.http.operation`
    /// span and of its outcome record.
    pub fn operation_id(&self) -> u64 {
        self.operation_id
    }

    /// The refusal, if the call was refused before transport (an invalid
    /// request, a full window, a spent ceiling, the in-flight bound, a
    /// closing client or no runtime). `None` once the attempt was spawned.
    pub fn refusal(&self) -> Option<&HttpError> {
        match &self.state {
            DispatchState::Refused(e) => e.as_ref(),
            _ => None,
        }
    }

    /// SHA-256 of the body sent, equal to the request's `wire()?.sha256()`;
    /// `None` only when the request failed validation, so no form existed.
    pub fn wire_sha256(&self) -> Option<[u8; 32]> {
        self.wire_sha256
    }

    /// The correlation token the calling handle carried, if any.
    pub fn correlation(&self) -> Option<&str> {
        self.correlation.as_deref()
    }

    /// Whether the attempt was spawned (not refused).
    pub fn is_in_flight(&self) -> bool {
        matches!(self.state, DispatchState::InFlight(_))
    }
}

impl<T> fmt::Debug for Dispatched<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dispatched")
            .field("operation_id", &self.operation_id)
            .field("correlation", &self.correlation)
            .field("wire_sha256", &self.wire_sha256.map(hex::encode))
            .field("refused", &matches!(self.state, DispatchState::Refused(_)))
            .finish()
    }
}

impl<T> Future for Dispatched<T> {
    type Output = Result<Response<T>, HttpError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match &mut this.state {
            DispatchState::Refused(error) => {
                let error = error
                    .take()
                    .expect("a Dispatched is polled once after Ready");
                this.state = DispatchState::Done;
                Poll::Ready(Err(error))
            }
            DispatchState::InFlight(receiver) => match Pin::new(receiver).poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(delivery)) => {
                    this.state = DispatchState::Done;
                    Poll::Ready(delivery.result)
                }
                Poll::Ready(Err(_)) => {
                    // The attempt task was torn down with its runtime before
                    // delivering: the request may have left the process.
                    this.state = DispatchState::Done;
                    Poll::Ready(Err(HttpError::new(
                        HttpErrorKind::Transport,
                        this.method,
                        this.endpoint,
                        TransportStage::Started,
                    )
                    .with_detail("the attempt task ended without delivering an outcome")))
                }
            },
            DispatchState::Done => panic!("a Dispatched is polled once after Ready"),
        }
    }
}

/// The broker identifiers a receipt carries, for correlation.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReceiptIds {
    /// The order ID of an order placement, modification or cancellation
    /// receipt.
    pub order_id: Option<OrderId>,
    /// The trigger ID of a GTT receipt.
    pub trigger_id: Option<u64>,
}

/// The record of one dispatched mutation's outcome, as delivered to the
/// outcome stream or kept in the orphan ring. It carries no request or
/// response body.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Outcome {
    /// The operation's ID, as on its span and its `Dispatched`.
    pub operation_id: u64,
    /// The correlation token the calling handle carried.
    pub correlation: Option<String>,
    /// HTTP method.
    pub method: Method,
    /// Endpoint template, never a concrete path.
    pub endpoint: Endpoint,
    /// SHA-256 of the body sent, equal to the request's `wire()?.sha256()`.
    pub wire_sha256: [u8; 32],
    /// How far the attempt got.
    pub stage: TransportStage,
    /// The normalized result, as the operation span and counters label it.
    pub result: HttpOperationResult,
    /// The error kind, for a failed attempt.
    pub error: Option<HttpErrorKind>,
    /// HTTP status, if a response was received.
    pub http_status: Option<u16>,
    /// The attempt number: always 1 for a mutation.
    pub attempt: u32,
    /// The broker identifiers the receipt carried; empty on failure and
    /// for mutations whose receipt has none.
    pub receipt: ReceiptIds,
    /// The client's source identity.
    pub source: SourceIdentity,
    /// When the attempt was dispatched.
    pub sent_at: Option<IngressStamp>,
    /// When the response head arrived, if one did.
    pub received_at: Option<IngressStamp>,
}

impl Outcome {
    /// The record of `result`, for the stream or the ring.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn of<T>(
        result: &Result<Response<T>, HttpError>,
        identity: &SourceIdentity,
        operation_id: u64,
        correlation: Option<String>,
        method: Method,
        endpoint: Endpoint,
        wire_sha256: [u8; 32],
        ids: fn(&T) -> ReceiptIds,
    ) -> Self {
        match result {
            Ok(response) => Self {
                operation_id,
                correlation,
                method,
                endpoint,
                wire_sha256,
                stage: TransportStage::ResponseReceived,
                result: HttpOperationResult::Ok,
                error: None,
                http_status: Some(response.meta.http_status),
                attempt: response.meta.attempt,
                receipt: ids(&response.data),
                source: response.meta.source.clone(),
                sent_at: Some(response.meta.sent_at),
                received_at: Some(response.meta.received_at),
            },
            Err(e) => Self {
                operation_id,
                correlation,
                method,
                endpoint,
                wire_sha256,
                stage: e.stage(),
                result: operation_result(e),
                error: Some(e.kind()),
                http_status: e.http_status(),
                attempt: e.attempt(),
                receipt: ReceiptIds::default(),
                source: e.source().cloned().unwrap_or_else(|| identity.clone()),
                sent_at: e.sent_at(),
                received_at: e.received_at(),
            },
        }
    }
}

/// The normalized result of a finished operation, as the span labels it.
pub(crate) fn operation_result(e: &HttpError) -> HttpOperationResult {
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

impl ReceiptIds {
    /// No identifiers: conversions, authorisations and session results.
    pub fn none<T>(_: &T) -> Self {
        Self::default()
    }

    /// The order ID of an order receipt.
    pub fn of_order(receipt: &crate::kite::connect::models::OrderReceipt) -> Self {
        Self {
            order_id: Some(receipt.order_id.clone()),
            trigger_id: None,
        }
    }

    /// The trigger ID of a GTT receipt.
    pub fn of_trigger(receipt: &crate::kite::connect::models::GttReceipt) -> Self {
        Self {
            order_id: None,
            trigger_id: Some(receipt.trigger_id),
        }
    }
}

/// Why the outcome stream ended before the client did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutcomeStreamError {
    /// The consumer fell behind by the stream's capacity (`B-HTTP-13`): the
    /// client closed the stream and kept counting; `dropped` outcomes have
    /// gone to the orphan ring so far.
    Overflow {
        /// Outcomes not delivered to the stream at the time of the report.
        dropped: u64,
    },
}

impl fmt::Display for OutcomeStreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow { dropped } => write!(
                f,
                "the outcome stream overflowed its capacity (B-HTTP-13); {dropped} outcome(s) not delivered"
            ),
        }
    }
}

impl std::error::Error for OutcomeStreamError {}

/// The receiving end of a client's outcome stream.
///
/// Every dispatched mutation's outcome arrives here exactly once, in
/// completion order, whether or not its `Dispatched` was awaited. The
/// stream is bounded (`B-HTTP-13`): a consumer that falls behind by the
/// capacity gets, after draining what was buffered, one `Err` naming how
/// many outcomes were not delivered, and then the end of the stream; the
/// client never blocks on the consumer and keeps counting the outcomes it
/// could not deliver.
pub struct OutcomeStream {
    receiver: tokio::sync::mpsc::Receiver<Outcome>,
    shared: std::sync::Arc<StreamShared>,
    reported: bool,
}

#[derive(Default)]
pub(crate) struct StreamShared {
    pub(crate) overflowed: std::sync::atomic::AtomicBool,
    pub(crate) dropped: std::sync::atomic::AtomicU64,
}

impl OutcomeStream {
    pub(crate) fn new(
        receiver: tokio::sync::mpsc::Receiver<Outcome>,
        shared: std::sync::Arc<StreamShared>,
    ) -> Self {
        Self {
            receiver,
            shared,
            reported: false,
        }
    }

    /// The next outcome; `Some(Err)` once if the stream overflowed, then
    /// `None`; `None` when the client has been dropped and every outcome
    /// delivered.
    pub async fn recv(&mut self) -> Option<Result<Outcome, OutcomeStreamError>> {
        use std::sync::atomic::Ordering;
        match self.receiver.recv().await {
            Some(outcome) => Some(Ok(outcome)),
            None => {
                if !self.reported && self.shared.overflowed.load(Ordering::Acquire) {
                    self.reported = true;
                    Some(Err(OutcomeStreamError::Overflow {
                        dropped: self.shared.dropped.load(Ordering::Acquire),
                    }))
                } else {
                    None
                }
            }
        }
    }

    /// Outcomes buffered and not yet received.
    pub fn len(&self) -> usize {
        self.receiver.len()
    }

    /// Whether nothing is buffered.
    pub fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }
}

impl fmt::Debug for OutcomeStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutcomeStream")
            .field("buffered", &self.receiver.len())
            .finish()
    }
}

/// `HTTPClient::close` reached its deadline with dispatches still in
/// flight. They finish on the runtime and deliver to the stream or ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseError {
    /// Dispatches still in flight when the deadline passed.
    pub in_flight: usize,
}

impl fmt::Display for CloseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the close deadline passed with {} dispatch(es) in flight",
            self.in_flight
        )
    }
}

impl std::error::Error for CloseError {}
