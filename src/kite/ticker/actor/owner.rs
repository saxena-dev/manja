//! The supervised, single-owner ticker.
//!
//! [`TickerBuilder::spawn`] starts one owner task per ticker instance and
//! returns three parts:
//!
//! - [`TickerHandle`]: shutdown and status. `Clone + Send + Sync`; every
//!   clone talks to the same owner.
//! - [`TickerEvents`]: the primary receiver, a
//!   `Stream<Item = Result<TickerEvent, TickerError>>`. Exactly one per
//!   instance; `Send + Unpin + 'static` and not `Clone`.
//! - [`TaskGuard`]: the supervised result of the owner task.
//!
//! # Ownership
//!
//! The owner task alone holds the socket, the connection epoch, the ingress
//! sequence and the termination decision. Nothing else can read or write the
//! socket: no handle exposes it, there is no shared socket mutex, and the
//! owner's state is reachable only through messages and a status snapshot.
//! It uses only the immutable [`Credentials`] snapshot it was built with.
//!
//! # Delivery
//!
//! Every binary and text application message is delivered as a
//! [`RawObservation`] before any interpretation, the one-byte heartbeat
//! included; ping, pong and frame-level traffic are not application
//! messages and are not delivered. [`LifecycleEvent`]s share the same queue
//! and the same source order: each event takes the next ingress sequence of
//! its connection epoch as it is emitted. Delivery transfers ownership; the
//! owner keeps no copy.
//!
//! The queue is bounded by message count (`B-TK-05`) and by retained payload
//! bytes (`B-TK-06`); a message over `B-TK-10` fails the connection. When the
//! queue is full the owner stops reading the socket and waits up to
//! `B-TK-08` for room, then fails with
//! [`TerminalReason::DeliveryOverload`]; it fails the same way when the
//! oldest queued event is older than `B-TK-07`. Nothing is silently
//! dropped. Status and shutdown never need room in the queue. See
//! [`crate::kite::ticker::actor::delivery`] for every bound, cancellation
//! boundary and teardown outcome.
//!
//! # Termination
//!
//! - [`TickerHandle::shutdown`] stops intake, closes the socket and delivers
//!   the remaining events within `B-TK-12`. Once the consumer has taken
//!   them, the stream ends with `None`: the only clean end of stream.
//! - Any other end is terminal: the stream yields the remaining events, then
//!   one `Err(TickerError)`, then `None`. It is reported once.
//! - Dropping [`TickerEvents`] or every [`TickerHandle`] terminates the
//!   owner; neither is a clean shutdown.
//! - Dropping the [`TaskGuard`] does not stop the owner. The outcome stays
//!   visible as the sticky [`TickerStatus::terminal`] reason.
//!
//! Subscription commands (see [`crate::kite::ticker::actor::subscriptions`])
//! go through a bounded mailbox (`B-TK-09`); a full mailbox is an immediate
//! [`CommandError::MailboxFull`], never an indefinite wait. They are
//! accepted in every state; on connecting, the owner writes the desired map
//! before it reports `Active`.
//!
//! Lost connections and failed attempts are retried within bounds; see
//! [`crate::kite::ticker::actor::lifecycle`] for every cause and its
//! disposition.
//!
//! # Cancellation
//!
//! Dropping a [`TickerHandle::shutdown`] future after its first poll does
//! not withdraw the request. Dropping a pending `next()` on
//! [`TickerEvents`] loses nothing: an event is removed from the queue only
//! when it is returned.
//!
use std::collections::BTreeMap;
use std::fmt;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::{FutureExt, SinkExt, Stream, StreamExt};
use secrecy::{ExposeSecret, Secret};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch, Notify};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::{self, protocol::WebSocketConfig, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::kite::connect::credentials::Credentials;
use crate::kite::envelope::{
    DisconnectReason, GapFacts, LifecycleEvent, LifecycleKind, PayloadKind, RawObservation,
    ReceiveTime, SourceIdentity, SourceKey, SourceSequencer, MAX_PAYLOAD_BYTES_LIMIT,
};
use crate::kite::obs::handle::GaugeGuard;
use crate::kite::obs::schema::{
    ConnectionResult, Decision, PayloadKindLabel, QueueRole, ReconnectReason, RestoreResult,
    ShutdownResult,
};
use crate::kite::obs::Observability;
use crate::kite::protocol::InstrumentToken;
use crate::kite::ticker::actor::delivery;
use crate::kite::ticker::actor::lifecycle::{self, Backoff, Disposition, ReconnectLimits};
pub use crate::kite::ticker::actor::status::TickerStatus;
use crate::kite::ticker::actor::status::{
    self, QueueGauges, QueueStatus, StatusCore, TickerFailure, TickerObs, Traffic,
};
use crate::kite::ticker::actor::subscriptions::{
    reconcile, DesiredSubscriptions, Revision, SubscriptionCommand, SubscriptionError,
    MAX_INSTRUMENTS_PER_CONNECTION,
};
use crate::kite::ticker::models::Mode;

/// The Kite Connect WebSocket endpoint
/// (`kite-api-docs/docs/connect/v3/websocket.md:20`).
pub const KITE_TICKER_URL: &str = "wss://ws.kite.trade";

// ---- limits -------------------------------------------------------------

/// A ticker bound outside its documented range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickerLimitError(pub &'static str);

impl fmt::Display for TickerLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is outside its documented range", self.0)
    }
}

impl std::error::Error for TickerLimitError {}

/// Runtime bounds of one ticker instance (SDK contract §5.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickerLimits {
    handshake_timeout: Duration,
    queue_messages: usize,
    queue_bytes: usize,
    delivery_wait: Duration,
    max_queue_age: Duration,
    command_mailbox: usize,
    max_payload: usize,
    max_instruments: usize,
    shutdown_deadline: Duration,
    reconnect: ReconnectLimits,
}

impl Default for TickerLimits {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(10),
            queue_messages: 4096,
            queue_bytes: 64 << 20,
            delivery_wait: Duration::from_secs(1),
            max_queue_age: Duration::from_secs(5),
            command_mailbox: 64,
            max_payload: 1 << 20,
            max_instruments: MAX_INSTRUMENTS_PER_CONNECTION,
            shutdown_deadline: Duration::from_secs(5),
            reconnect: ReconnectLimits::default(),
        }
    }
}

fn within<T: PartialOrd>(id: &'static str, v: T, min: T, max: T) -> Result<T, TickerLimitError> {
    if v < min || v > max {
        Err(TickerLimitError(id))
    } else {
        Ok(v)
    }
}

impl TickerLimits {
    /// Handshake timeout (`B-TK-01`, 1 s to 60 s).
    pub fn with_handshake_timeout(mut self, d: Duration) -> Result<Self, TickerLimitError> {
        self.handshake_timeout = within(
            "B-TK-01",
            d,
            Duration::from_secs(1),
            Duration::from_secs(60),
        )?;
        Ok(self)
    }

    /// Primary queue capacity in messages (`B-TK-05`, 16 to 65 536).
    pub fn with_queue_messages(mut self, n: usize) -> Result<Self, TickerLimitError> {
        self.queue_messages = within("B-TK-05", n, 16, 65_536)?;
        Ok(self)
    }

    /// Primary queue retained bytes (`B-TK-06`, 1 MiB to 1 GiB, at least the
    /// payload bound).
    pub fn with_queue_bytes(mut self, n: usize) -> Result<Self, TickerLimitError> {
        self.queue_bytes = within("B-TK-06", n, 1 << 20, 1 << 30)?;
        self.check()
    }

    /// Primary delivery wait (`B-TK-08`, 10 ms to 30 s, at most the oldest
    /// queued age).
    pub fn with_delivery_wait(mut self, d: Duration) -> Result<Self, TickerLimitError> {
        self.delivery_wait = within(
            "B-TK-08",
            d,
            Duration::from_millis(10),
            Duration::from_secs(30),
        )?;
        self.check()
    }

    /// Oldest queued age (`B-TK-07`, 100 ms to 60 s, at least the delivery
    /// wait).
    pub fn with_max_queue_age(mut self, d: Duration) -> Result<Self, TickerLimitError> {
        self.max_queue_age = within(
            "B-TK-07",
            d,
            Duration::from_millis(100),
            Duration::from_secs(60),
        )?;
        self.check()
    }

    /// Command mailbox capacity (`B-TK-09`, 1 to 1 024).
    pub fn with_command_mailbox(mut self, n: usize) -> Result<Self, TickerLimitError> {
        self.command_mailbox = within("B-TK-09", n, 1, 1024)?;
        Ok(self)
    }

    /// Maximum payload per message (`B-TK-10`, 64 KiB to 16 MiB).
    pub fn with_max_payload(mut self, n: usize) -> Result<Self, TickerLimitError> {
        self.max_payload = within("B-TK-10", n, 64 << 10, MAX_PAYLOAD_BYTES_LIMIT)?;
        self.check()
    }

    /// Desired instruments per connection (`B-TK-11`, 1 to 3 000).
    pub fn with_max_instruments(mut self, n: usize) -> Result<Self, TickerLimitError> {
        self.max_instruments = within("B-TK-11", n, 1, MAX_INSTRUMENTS_PER_CONNECTION)?;
        Ok(self)
    }

    /// Shutdown deadline (`B-TK-12`, 100 ms to 60 s).
    pub fn with_shutdown_deadline(mut self, d: Duration) -> Result<Self, TickerLimitError> {
        self.shutdown_deadline = within(
            "B-TK-12",
            d,
            Duration::from_millis(100),
            Duration::from_secs(60),
        )?;
        Ok(self)
    }

    /// Reconnect, backoff and liveness bounds (`B-TK-02` to `B-TK-04`).
    pub fn with_reconnect(mut self, reconnect: ReconnectLimits) -> Self {
        self.reconnect = reconnect;
        self
    }

    fn check(self) -> Result<Self, TickerLimitError> {
        if self.queue_bytes < self.max_payload {
            return Err(TickerLimitError("B-TK-06 >= B-TK-10"));
        }
        if self.delivery_wait > self.max_queue_age {
            return Err(TickerLimitError("B-TK-08 <= B-TK-07"));
        }
        Ok(self)
    }

    /// `B-TK-01`.
    pub fn handshake_timeout(&self) -> Duration {
        self.handshake_timeout
    }
    /// `B-TK-05`.
    pub fn queue_messages(&self) -> usize {
        self.queue_messages
    }
    /// `B-TK-06`.
    pub fn queue_bytes(&self) -> usize {
        self.queue_bytes
    }
    /// `B-TK-08`.
    pub fn delivery_wait(&self) -> Duration {
        self.delivery_wait
    }
    /// `B-TK-07`.
    pub fn max_queue_age(&self) -> Duration {
        self.max_queue_age
    }
    /// `B-TK-09`.
    pub fn command_mailbox(&self) -> usize {
        self.command_mailbox
    }
    /// `B-TK-10`.
    pub fn max_payload(&self) -> usize {
        self.max_payload
    }
    /// `B-TK-11`.
    pub fn max_instruments(&self) -> usize {
        self.max_instruments
    }
    /// `B-TK-12`.
    pub fn shutdown_deadline(&self) -> Duration {
        self.shutdown_deadline
    }
    /// `B-TK-02` to `B-TK-04`.
    pub fn reconnect(&self) -> &ReconnectLimits {
        &self.reconnect
    }
}

// ---- public types -------------------------------------------------------

/// One item of the primary stream, in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TickerEvent {
    /// An application message exactly as received.
    Raw(RawObservation),
    /// A fact about the source.
    Lifecycle(LifecycleEvent),
}

/// Connection state (architecture §7.1). `Active` is not quote freshness
/// and not permission to trade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TickerState {
    /// Not connected.
    Disconnected,
    /// A connection attempt is in progress.
    Connecting,
    /// Restoring subscriptions on a new connection.
    Restoring,
    /// Connected, with subscriptions restored.
    Active,
    /// Waiting before the next attempt.
    Backoff,
    /// The broker rejected the credentials; terminal.
    AuthRejected,
    /// Shutting down.
    Stopping,
    /// Stopped after a requested shutdown; terminal.
    Stopped,
    /// Failed; terminal.
    Failed,
}

/// Why a ticker instance ended.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TerminalReason {
    /// A requested shutdown completed and every event was delivered. The
    /// only clean end.
    Shutdown,
    /// The handshake was rejected with 401 or 403.
    AuthRejected {
        /// HTTP status of the rejected handshake.
        http_status: u16,
    },
    /// The handshake was rejected with another HTTP status.
    HandshakeRejected {
        /// HTTP status of the rejected handshake.
        http_status: u16,
    },
    /// The handshake did not finish within `B-TK-01`.
    HandshakeTimeout,
    /// The connection or handshake failed before any response.
    ConnectFailed,
    /// An established connection ended.
    Disconnected(DisconnectReason),
    /// The consumer did not make room in the primary queue within
    /// `B-TK-08`.
    DeliveryOverload,
    /// The primary receiver was dropped.
    ReceiverDropped {
        /// Events accepted but not delivered.
        undelivered: usize,
    },
    /// Every [`TickerHandle`] was dropped.
    HandlesDropped,
    /// The close handshake failed during shutdown.
    CloseFailed,
    /// Shutdown interrupted a subscription write: the broker may have
    /// received none, part or all of it.
    SendInterrupted {
        /// The revision being written.
        revision: u64,
    },
    /// Shutdown did not deliver every event within `B-TK-12`.
    ShutdownDeadlineExpired {
        /// Events the owner had accepted but could not deliver.
        undelivered: usize,
    },
    /// Every connection attempt of an outage failed, or its time ran out
    /// (`B-TK-02`).
    ReconnectExhausted {
        /// Failed attempts in the outage.
        attempts: u32,
        /// The last failure.
        last: Box<TerminalReason>,
    },
    /// The owner task panicked.
    Panicked,
    /// The owner task was aborted, for example by runtime shutdown.
    Aborted,
}

impl TerminalReason {
    /// Whether this is the clean end of a requested shutdown.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Shutdown)
    }
}

impl fmt::Display for TerminalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shutdown => f.write_str("the ticker shut down"),
            Self::AuthRejected { http_status } => {
                write!(
                    f,
                    "the handshake was rejected as unauthorized ({http_status})"
                )
            }
            Self::HandshakeRejected { http_status } => {
                write!(f, "the handshake was rejected with HTTP {http_status}")
            }
            Self::HandshakeTimeout => f.write_str("the handshake timed out"),
            Self::ConnectFailed => f.write_str("the connection could not be established"),
            Self::Disconnected(r) => write!(f, "the connection ended: {r:?}"),
            Self::DeliveryOverload => {
                f.write_str("the consumer did not accept events within the delivery wait")
            }
            Self::ReceiverDropped { undelivered } => write!(
                f,
                "the primary receiver was dropped with {undelivered} events undelivered"
            ),
            Self::HandlesDropped => f.write_str("every ticker handle was dropped"),
            Self::CloseFailed => f.write_str("the close handshake failed"),
            Self::SendInterrupted { revision } => write!(
                f,
                "shutdown interrupted the write of revision {revision}; its delivery is unknown"
            ),
            Self::ShutdownDeadlineExpired { undelivered } => {
                write!(f, "shutdown expired with {undelivered} events undelivered")
            }
            Self::ReconnectExhausted { attempts, last } => write!(
                f,
                "reconnecting failed after {attempts} attempts; last: {last}"
            ),
            Self::Panicked => f.write_str("the ticker task panicked"),
            Self::Aborted => f.write_str("the ticker task was aborted"),
        }
    }
}

/// The terminal error of the primary stream, yielded once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickerError(TerminalReason);

impl TickerError {
    /// Why the ticker ended.
    pub fn reason(&self) -> &TerminalReason {
        &self.0
    }
}

impl fmt::Display for TickerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for TickerError {}

/// The supervised result of an owner task.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskOutcome {
    /// A requested shutdown completed cleanly.
    Clean,
    /// The ticker ended for a terminal reason other than those below.
    Terminal(TerminalReason),
    /// The owner task panicked.
    Panicked,
    /// Shutdown expired before every event was delivered.
    DeadlineExpired {
        /// Events not delivered.
        undelivered: usize,
    },
}

impl From<TerminalReason> for TaskOutcome {
    fn from(r: TerminalReason) -> Self {
        match r {
            TerminalReason::Shutdown => Self::Clean,
            TerminalReason::Panicked => Self::Panicked,
            TerminalReason::ShutdownDeadlineExpired { undelivered } => {
                Self::DeadlineExpired { undelivered }
            }
            other => Self::Terminal(other),
        }
    }
}

/// Why a ticker could not be started.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TickerSpawnError {
    /// Not called from within a Tokio runtime.
    NoRuntime,
    /// The URL is not a `ws://` or `wss://` URL without query or fragment.
    InvalidUrl,
}

impl fmt::Display for TickerSpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoRuntime => "a ticker must be spawned within a Tokio runtime",
            Self::InvalidUrl => "the ticker URL must be ws:// or wss:// with no query",
        })
    }
}

impl std::error::Error for TickerSpawnError {}

// ---- shared state -------------------------------------------------------

// Commands to the owner. When every sender is dropped, every handle is
// gone.
pub(crate) enum Command {
    Subscription {
        command: SubscriptionCommand,
        reply: oneshot::Sender<Result<Revision, SubscriptionError>>,
        span: tracing::Span,
    },
}

/// Why a subscription command did not complete.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommandError {
    /// The owner refused the command; nothing changed.
    Invalid(SubscriptionError),
    /// The command mailbox (`B-TK-09`) is full; the command was not
    /// submitted.
    MailboxFull,
    /// The ticker has ended.
    Terminated(TerminalReason),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "the command was refused: {e}"),
            Self::MailboxFull => f.write_str("the command mailbox is full"),
            Self::Terminated(r) => write!(f, "the ticker has ended: {r}"),
        }
    }
}

impl std::error::Error for CommandError {}

struct Shared {
    status: watch::Sender<StatusCore>,
    traffic: Traffic,
    queue: delivery::Stats,
    // The command mailbox's gauge contribution, when recording.
    command_gauges: Option<QueueGauges>,
    stop: Notify,
    stop_requested: AtomicBool,
}

impl Shared {
    fn update(&self, f: impl FnOnce(&mut StatusCore)) {
        self.status.send_modify(|s| {
            f(s);
            s.snapshot_revision += 1;
        });
    }

    fn terminate(&self, state: TickerState, reason: TerminalReason) {
        self.update(|s| {
            s.state = state;
            if s.terminal.is_none() {
                s.terminal = Some(reason);
            }
        });
    }

    fn publish_commands(&self, commands: &mpsc::Sender<Command>) {
        if let Some(g) = &self.command_gauges {
            g.set(commands.max_capacity() - commands.capacity(), 0, None);
        }
    }

    fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
        self.stop.notify_waiters();
    }

    async fn stopped(&self) {
        loop {
            let notified = self.stop.notified();
            if self.stop_requested.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

// ---- builder ------------------------------------------------------------

/// Builder for a ticker instance.
#[must_use]
pub struct TickerBuilder {
    credentials: Credentials,
    url: String,
    limits: TickerLimits,
    identity: Option<SourceIdentity>,
    observability: Observability,
    #[cfg(test)]
    faults: Faults,
}

impl fmt::Debug for TickerBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TickerBuilder")
            .field("credentials", &self.credentials)
            .field("url", &self.url)
            .field("limits", &self.limits)
            .finish()
    }
}

impl TickerBuilder {
    /// A ticker for `credentials`, connecting to [`KITE_TICKER_URL`].
    pub fn new(credentials: Credentials) -> Self {
        Self {
            credentials,
            url: KITE_TICKER_URL.to_string(),
            limits: TickerLimits::default(),
            identity: None,
            observability: Observability::disabled(),
            #[cfg(test)]
            faults: Faults::default(),
        }
    }

    /// Connect to `url` instead, a `ws://` or `wss://` URL with no query.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    /// Use `limits`.
    pub fn limits(mut self, limits: TickerLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Stamp observations with `identity`; a fresh identity by default.
    pub fn identity(mut self, identity: SourceIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Record in `obs`'s scope; disabled by default.
    pub fn observability(mut self, obs: Observability) -> Self {
        self.observability = obs;
        self
    }

    /// Start the owner task on the current Tokio runtime.
    pub fn spawn(self) -> Result<(TickerHandle, TickerEvents, TaskGuard), TickerSpawnError> {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| TickerSpawnError::NoRuntime)?;
        let url = self.url.trim_end_matches('/');
        let valid =
            (url.starts_with("ws://") || url.starts_with("wss://")) && !url.contains(['?', '#']);
        if !valid {
            return Err(TickerSpawnError::InvalidUrl);
        }
        // `wss://ws.kite.trade?api_key=…` is sent as `GET /?api_key=…`.
        let has_path = url
            .split_once("://")
            .is_some_and(|(_, rest)| rest.contains('/'));
        let url = Secret::new(format!(
            "{url}{}?{}",
            if has_path { "" } else { "/" },
            self.credentials.websocket_query().expose()
        ));
        let identity = self.identity.unwrap_or_else(SourceIdentity::generate);
        let obs = TickerObs {
            obs: self.observability,
            feed_id: identity.feed_id().to_string(),
        };
        let (status, status_rx) = watch::channel(StatusCore::new());
        let (commands_tx, commands) = mpsc::channel(self.limits.command_mailbox);
        let (events_tx, events_rx) = delivery::queue(
            self.limits.queue_messages,
            self.limits.queue_bytes,
            obs.queue_gauges(QueueRole::RawPrimary),
        );
        let shared = Arc::new(Shared {
            status,
            traffic: Traffic::new(),
            queue: events_tx.stats(),
            command_gauges: obs.queue_gauges(QueueRole::Command),
            stop: Notify::new(),
            stop_requested: AtomicBool::new(false),
        });
        let keep_open = events_tx.keep_open();
        let owner = Owner {
            url: Arc::new(url),
            desired: DesiredSubscriptions::new(self.limits.max_instruments),
            sent: None,
            unsent: Vec::new(),
            limits: self.limits.clone(),
            sequencer: SourceSequencer::new(identity),
            shared: shared.clone(),
            commands,
            events: events_tx,
            interrupted: None,
            stop_at: None,
            last_sequence: None,
            gap: None,
            recovered: false,
            backoff: Backoff::new(&self.limits.reconnect),
            pending_gauges: obs.queue_gauges(QueueRole::PendingSend),
            connected: None,
            obs: obs.clone(),
            #[cfg(test)]
            faults: self.faults,
        };
        let supervisor = shared.clone();
        let task = runtime.spawn(async move {
            // Held until the terminal reason is recorded, so the receiver
            // never sees the end of the queue before the reason.
            let _keep_open = keep_open;
            let reason = match AssertUnwindSafe(owner.run()).catch_unwind().await {
                Ok(reason) => reason,
                Err(_) => {
                    supervisor.terminate(TickerState::Failed, TerminalReason::Panicked);
                    obs.shutdown(ShutdownResult::Panicked, Duration::ZERO);
                    TerminalReason::Panicked
                }
            };
            TaskOutcome::from(reason)
        });
        Ok((
            TickerHandle {
                shared: shared.clone(),
                status: status_rx,
                commands: commands_tx,
            },
            TickerEvents {
                rx: events_rx,
                shared,
                done: false,
            },
            TaskGuard { task },
        ))
    }
}

// ---- handles ------------------------------------------------------------

/// Shutdown and status for one ticker instance. `Clone + Send + Sync`.
///
/// Dropping the last handle terminates the owner with
/// [`TerminalReason::HandlesDropped`].
#[derive(Clone)]
pub struct TickerHandle {
    shared: Arc<Shared>,
    status: watch::Receiver<StatusCore>,
    commands: mpsc::Sender<Command>,
}

impl fmt::Debug for TickerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TickerHandle")
            .field("status", &self.status())
            .finish()
    }
}

impl TickerHandle {
    /// The current status snapshot. Never waits on the data queue.
    pub fn status(&self) -> TickerStatus {
        let core = self.status.borrow().clone();
        let (messages, retained_bytes, oldest) = self.shared.queue.read();
        let queue = QueueStatus {
            messages,
            retained_bytes,
            oldest_age: oldest.map(|t| t.elapsed()),
            commands: self.commands.max_capacity() - self.commands.capacity(),
            pending_send: self.shared.traffic.pending_send.load(Ordering::Relaxed),
        };
        self.shared.traffic.snapshot(&core, queue)
    }

    /// Wait until the snapshot revision exceeds `since`, then return the
    /// latest snapshot.
    ///
    /// Notifications coalesce: the returned `snapshot_revision` minus
    /// `since` is the number of changes the caller is receiving at once.
    /// This is best-effort status, not an event record; see
    /// [`crate::kite::ticker::actor::status`].
    pub async fn changed(&self, since: u64) -> TickerStatus {
        let mut status = self.status.clone();
        let _ = status
            .wait_for(|s| s.snapshot_revision > since || s.terminal.is_some())
            .await;
        self.status()
    }

    /// Desire `tokens` in `mode`. A token already desired takes `mode`.
    ///
    /// Completes when the owner has accepted the command and assigned its
    /// revision; that is not broker acknowledgement. See
    /// [`crate::kite::ticker::actor::subscriptions`].
    pub async fn subscribe(
        &self,
        tokens: impl IntoIterator<Item = InstrumentToken>,
        mode: Mode,
    ) -> Result<Revision, CommandError> {
        self.command(SubscriptionCommand::Subscribe {
            tokens: tokens.into_iter().collect(),
            mode,
        })
        .await
    }

    /// Stop desiring `tokens`; tokens not desired are ignored.
    pub async fn unsubscribe(
        &self,
        tokens: impl IntoIterator<Item = InstrumentToken>,
    ) -> Result<Revision, CommandError> {
        self.command(SubscriptionCommand::Unsubscribe {
            tokens: tokens.into_iter().collect(),
        })
        .await
    }

    /// Change the mode of desired `tokens`. A token not desired is refused;
    /// this never subscribes.
    pub async fn set_mode(
        &self,
        tokens: impl IntoIterator<Item = InstrumentToken>,
        mode: Mode,
    ) -> Result<Revision, CommandError> {
        self.command(SubscriptionCommand::SetMode {
            tokens: tokens.into_iter().collect(),
            mode,
        })
        .await
    }

    /// Replace the whole desired map atomically.
    pub async fn replace(
        &self,
        desired: impl IntoIterator<Item = (InstrumentToken, Mode)>,
    ) -> Result<Revision, CommandError> {
        self.command(SubscriptionCommand::Replace {
            desired: desired.into_iter().collect(),
        })
        .await
    }

    /// Submit `command` without waiting for mailbox room, then wait for the
    /// owner's decision. Once submitted, dropping this future does not
    /// withdraw the command; [`TickerStatus::desired_revision`] shows
    /// whether it was applied.
    pub async fn command(&self, command: SubscriptionCommand) -> Result<Revision, CommandError> {
        let (reply, decision) = oneshot::channel();
        // Created in the caller's context and carried to the owner.
        let span = tracing::debug_span!(
            "manja.ticker.command",
            command = status::command_kind(&command).as_str(),
            revision = tracing::field::Empty,
            decision = tracing::field::Empty,
            rejection = tracing::field::Empty,
        );
        self.commands
            .try_send(Command::Subscription {
                command,
                reply,
                span,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => CommandError::MailboxFull,
                mpsc::error::TrySendError::Closed(_) => self.terminated(),
            })?;
        self.shared.publish_commands(&self.commands);
        match decision.await {
            Ok(result) => result.map_err(CommandError::Invalid),
            Err(_) => Err(self.terminated()),
        }
    }

    fn terminated(&self) -> CommandError {
        CommandError::Terminated(
            self.status
                .borrow()
                .terminal
                .clone()
                .unwrap_or(TerminalReason::Aborted),
        )
    }

    /// Request shutdown and wait until the owner has ended.
    ///
    /// Returns `Ok` for a clean shutdown and otherwise the terminal reason
    /// that ended the ticker. The request is made on the first poll;
    /// dropping the future afterwards does not withdraw it. A clean
    /// shutdown completes only when the consumer has room for the final
    /// events, within `B-TK-12`.
    pub async fn shutdown(&self) -> Result<(), TickerError> {
        self.shared.request_stop();
        let mut status = self.status.clone();
        let terminal = status
            .wait_for(|s| s.terminal.is_some())
            .await
            .map(|s| s.terminal.clone())
            .ok()
            .flatten()
            .unwrap_or(TerminalReason::Aborted);
        if terminal.is_clean() {
            Ok(())
        } else {
            Err(TickerError(terminal))
        }
    }
}

/// The primary receiver: raw observations and lifecycle events in source
/// order.
///
/// `Send + Unpin + 'static`. There is exactly one per instance; it is not
/// `Clone`, so a second consumer cannot exist:
///
/// ```compile_fail
/// # fn f(events: manja::kite::ticker::actor::owner::TickerEvents) {
/// let second = events.clone();
/// # }
/// ```
///
/// It yields `None` only after a clean shutdown; any other end yields one
/// `Err` first. Dropping it terminates the owner with
/// [`TerminalReason::ReceiverDropped`].
pub struct TickerEvents {
    rx: delivery::Receiver,
    shared: Arc<Shared>,
    done: bool,
}

impl fmt::Debug for TickerEvents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TickerEvents")
            .field("queued", &self.rx.len())
            .finish()
    }
}

impl Stream for TickerEvents {
    type Item = Result<TickerEvent, TickerError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(event)) => Poll::Ready(Some(Ok(event))),
            Poll::Ready(None) => {
                self.done = true;
                let terminal = self
                    .shared
                    .status
                    .borrow()
                    .terminal
                    .clone()
                    .unwrap_or(TerminalReason::Aborted);
                Poll::Ready((!terminal.is_clean()).then_some(Err(TickerError(terminal))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// The supervised owner task. Dropping it does not stop the task.
#[derive(Debug)]
pub struct TaskGuard {
    task: JoinHandle<TaskOutcome>,
}

impl TaskGuard {
    /// Wait for the owner task to end.
    pub async fn join(self) -> TaskOutcome {
        match self.task.await {
            Ok(outcome) => outcome,
            Err(e) if e.is_panic() => TaskOutcome::Panicked,
            Err(_) => TaskOutcome::Terminal(TerminalReason::Aborted),
        }
    }
}

// ---- the owner ----------------------------------------------------------

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

// Test-only fault seams.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Faults {
    /// Panic when this many raw observations have been delivered.
    pub(crate) panic_after: Option<usize>,
    /// Fail this many subscription writes.
    pub(crate) fail_sends: usize,
    /// Never complete a subscription write.
    pub(crate) stall_sends: bool,
}

struct Owner {
    url: Arc<Secret<String>>,
    desired: DesiredSubscriptions,
    // What the current connection was sent; `None` when not connected.
    sent: Option<BTreeMap<InstrumentToken, Mode>>,
    // Accepted revisions not yet written to a connection.
    unsent: Vec<Revision>,
    limits: TickerLimits,
    sequencer: SourceSequencer,
    shared: Arc<Shared>,
    commands: mpsc::Receiver<Command>,
    events: delivery::Sender,
    // A subscription write that shutdown interrupted.
    interrupted: Option<Revision>,
    // Set when shutdown was requested: every remaining delivery must
    // finish by then.
    stop_at: Option<Instant>,
    // Last ingress sequence issued in the current epoch.
    last_sequence: Option<u64>,
    // The loss a reconnect has yet to report as a gap.
    gap: Option<GapFacts>,
    // Whether the current outage has reached `Active` since it began.
    recovered: bool,
    backoff: Backoff,
    obs: TickerObs,
    // The pending-send gauge contribution, when recording.
    pending_gauges: Option<QueueGauges>,
    // The established-socket gauge contribution of the current connection.
    connected: Option<GaugeGuard>,
    #[cfg(test)]
    faults: Faults,
}

// Why a connection phase ended.
enum End {
    Shutdown,
    Terminal(TerminalReason),
    // An established connection ended.
    Lost(DisconnectReason),
    // A connection attempt failed.
    Rejected(TerminalReason),
}

impl Owner {
    async fn run(mut self) -> TerminalReason {
        let mut outage_started = Instant::now();
        let mut failures: u32 = 0;
        let mut cause: Option<ReconnectReason> = None;
        loop {
            if let Some(reason) = cause {
                // A reconnect is counted when its attempt starts.
                self.obs.reconnect(reason);
            }
            let (end, socket) = self.connection(failures + 1, cause).await;
            self.connected = None;
            let failure = match end {
                End::Shutdown | End::Terminal(_) => return self.finish(end, socket).await,
                End::Lost(reason) => {
                    if std::mem::take(&mut self.recovered) {
                        // A new outage begins with this loss.
                        outage_started = Instant::now();
                        failures = 0;
                    }
                    self.gap = Some(GapFacts {
                        previous_epoch: self.sequencer.epoch(),
                        last_sequence_in_previous_epoch: self.last_sequence,
                        disconnected_at: self.sequencer.elapsed_nanos_now(),
                        reconnected_at: self.sequencer.elapsed_nanos_now(),
                        reason,
                    });
                    let failure = TerminalReason::Disconnected(reason);
                    if lifecycle::after_disconnect(reason) == Disposition::Terminal {
                        return self.finish(End::Terminal(failure), socket).await;
                    }
                    failure
                }
                End::Rejected(reason) => {
                    if lifecycle::after_handshake(&reason) == Disposition::Terminal {
                        return self.finish(End::Terminal(reason), socket).await;
                    }
                    reason
                }
            };
            drop(socket);
            self.sent = None;
            self.record_failure(failure.clone());
            cause = Some(status::reconnect_reason(&failure));
            failures += 1;
            let limits = self.limits.reconnect.clone();
            let delay = self.backoff.delay(failures);
            let resume_at = Instant::now() + delay;
            if failures >= limits.attempts()
                || resume_at >= outage_started + limits.outage_deadline()
            {
                let exhausted = TerminalReason::ReconnectExhausted {
                    attempts: failures,
                    last: Box::new(failure),
                };
                return self.finish(End::Terminal(exhausted), None).await;
            }
            if let Err(end) = self.backoff_wait(delay, failures + 1).await {
                return self.finish(end, None).await;
            }
        }
    }

    // Wait out a backoff delay, still accepting commands; shutdown
    // interrupts it.
    async fn backoff_wait(&mut self, delay: Duration, next_attempt: u32) -> Result<(), End> {
        self.state(TickerState::Backoff);
        self.lifecycle(LifecycleKind::Backoff {
            delay_ms: delay.as_millis().min(u64::MAX as u128) as u64,
            next_attempt,
        })
        .await?;
        let resume = tokio::time::sleep(delay);
        tokio::pin!(resume);
        let shared = self.shared.clone();
        loop {
            tokio::select! {
                biased;
                _ = shared.stopped() => return Err(End::Shutdown),
                _ = self.events.closed() => return Err(self.receiver_dropped(0)),
                command = self.commands.recv() => match command {
                    None => return Err(End::Terminal(TerminalReason::HandlesDropped)),
                    Some(c) => self.on_command(c),
                },
                _ = &mut resume => return Ok(()),
            }
        }
    }

    fn next_key(&mut self) -> SourceKey {
        let key = self.sequencer.next_key();
        self.last_sequence = Some(key.ingress_sequence());
        key
    }

    fn state(&self, state: TickerState) {
        let epoch = self.sequencer.epoch();
        self.shared.update(|s| {
            s.state = state;
            s.connection_epoch = epoch;
        });
    }

    fn lifecycle_event(&mut self, kind: LifecycleKind) -> TickerEvent {
        let key = self.next_key();
        TickerEvent::Lifecycle(LifecycleEvent::new(
            key,
            ReceiveTime::now(),
            self.sequencer.elapsed_nanos_now(),
            kind,
        ))
    }

    async fn lifecycle(&mut self, kind: LifecycleKind) -> Result<(), End> {
        let event = self.lifecycle_event(kind);
        self.deliver(event).await
    }

    // Queue one event, waiting for room: up to `B-TK-08` normally, and up
    // to the shutdown deadline once shutdown was requested.
    async fn deliver(&mut self, event: TickerEvent) -> Result<(), End> {
        let room = self.events.reserve(delivery::charge(&event));
        let shared = self.shared.clone();
        tokio::pin!(room);
        let normal_until = Instant::now() + self.limits.delivery_wait;
        loop {
            let until = self.stop_at.unwrap_or(normal_until);
            tokio::select! {
                biased;
                r = &mut room => {
                    return match r {
                        Ok(room) => {
                            room.send(event);
                            Ok(())
                        }
                        Err(delivery::Closed) => Err(self.receiver_dropped(1)),
                    };
                }
                _ = shared.stopped(), if self.stop_at.is_none() => {
                    self.stop_at = Some(Instant::now() + self.limits.shutdown_deadline);
                }
                _ = tokio::time::sleep_until(until) => {
                    return Err(match self.stop_at {
                        Some(_) => End::Terminal(TerminalReason::ShutdownDeadlineExpired {
                            undelivered: 1,
                        }),
                        None => End::Terminal(TerminalReason::DeliveryOverload),
                    });
                }
            }
        }
    }

    fn receiver_dropped(&self, pending: usize) -> End {
        End::Terminal(TerminalReason::ReceiverDropped {
            undelivered: self.events.len() + pending,
        })
    }

    // The consumer is too slow when the oldest queued event is older than
    // `B-TK-07`.
    fn queue_age_deadline(&self) -> Option<Instant> {
        self.events.oldest().map(|t| t + self.limits.max_queue_age)
    }

    // One connection attempt under a fresh epoch: connect, restore, then
    // serve until the connection ends.
    async fn connection(
        &mut self,
        attempt: u32,
        cause: Option<ReconnectReason>,
    ) -> (End, Option<Socket>) {
        // The lost epoch may have issued more events (its backoff) since
        // the loss.
        if let Some(gap) = self.gap.as_mut() {
            if gap.previous_epoch == self.sequencer.epoch() {
                gap.last_sequence_in_previous_epoch = self.last_sequence;
            }
        }
        self.sequencer.begin_epoch();
        self.last_sequence = None;
        self.state(TickerState::Connecting);
        if let Err(end) = self
            .lifecycle(LifecycleKind::ConnectAttempt { attempt })
            .await
        {
            return (end, None);
        }
        let shared = self.shared.clone();
        let span = tracing::debug_span!(
            "manja.ticker.connection",
            feed_id = self.obs.feed_id.as_str(),
            connection_epoch = self.sequencer.epoch().0,
            reason = cause.map_or("start", ReconnectReason::as_str),
            result = tracing::field::Empty,
        );
        let started = Instant::now();
        let obs = self.obs.clone();
        let finished = |result: ConnectionResult| {
            span.record("result", result.as_str());
            obs.connection_attempt(result, started.elapsed());
        };
        let attempt = connect(self.url.clone(), self.limits.clone());
        tokio::pin!(attempt);
        // Commands are accepted while connecting; restoration sends them.
        let connected = loop {
            tokio::select! {
                biased;
                _ = shared.stopped() => {
                    finished(ConnectionResult::Cancelled);
                    return (End::Shutdown, None);
                }
                _ = self.events.closed() => return (self.receiver_dropped(0), None),
                command = self.commands.recv() => match command {
                    None => return (End::Terminal(TerminalReason::HandlesDropped), None),
                    Some(c) => self.on_command(c),
                },
                r = &mut attempt => break r,
            }
        };
        let mut socket = match connected {
            Ok(socket) => {
                finished(ConnectionResult::Ok);
                self.connected = Some(self.obs.connected());
                socket
            }
            Err(reason) => {
                finished(status::connection_result(&reason));
                if let TerminalReason::AuthRejected { http_status } = reason {
                    // Terminal: the ticker never retries rejected
                    // credentials.
                    let _ = self
                        .lifecycle(LifecycleKind::AuthRejected { http_status })
                        .await;
                }
                return (End::Rejected(reason), None);
            }
        };
        if let Err(end) = self.lifecycle(LifecycleKind::Connected).await {
            return (end, Some(socket));
        }
        if let Some(mut gap) = self.gap.take() {
            gap.reconnected_at = self.sequencer.elapsed_nanos_now();
            if let Err(end) = self.lifecycle(LifecycleKind::Gap(gap)).await {
                return (end, Some(socket));
            }
        }
        // Restore the desired map before reporting `Active`.
        self.state(TickerState::Restoring);
        self.sent = Some(BTreeMap::new());
        let restore = tracing::debug_span!(
            "manja.ticker.restore",
            connection_epoch = self.sequencer.epoch().0,
            desired_revision = self.desired.revision().0,
            instrument_count = self.desired.map().len(),
            sent_count = reconcile(&BTreeMap::new(), self.desired.map()).len(),
            result = tracing::field::Empty,
        );
        let started = Instant::now();
        let restored = self.sync(&mut socket).await;
        let result = match &restored {
            Ok(()) => RestoreResult::Sent,
            Err(End::Lost(_)) => RestoreResult::SendFailed,
            Err(_) => RestoreResult::Cancelled,
        };
        restore.record("result", result.as_str());
        self.obs.restore(result, started.elapsed());
        if let Err(end) = restored {
            return (end, Some(socket));
        }
        let revision = self.desired.revision().0;
        if let Err(end) = self.lifecycle(LifecycleKind::Active { revision }).await {
            return (end, Some(socket));
        }
        self.state(TickerState::Active);
        self.recovered = true;
        let liveness = self.limits.reconnect.liveness_timeout();
        let mut last_seen = Instant::now();
        loop {
            let age_deadline = self.queue_age_deadline();
            let end = tokio::select! {
                biased;
                _ = shared.stopped() => End::Shutdown,
                _ = self.events.closed() => self.receiver_dropped(0),
                command = self.commands.recv() => match command {
                    None => End::Terminal(TerminalReason::HandlesDropped),
                    Some(c) => {
                        self.on_command(c);
                        // Apply everything already queued, then send once.
                        while let Ok(c) = self.commands.try_recv() {
                            self.on_command(c);
                        }
                        match self.sync(&mut socket).await {
                            Ok(()) => continue,
                            Err(end) => end,
                        }
                    }
                },
                message = socket.next() => {
                    // Any traffic, pings and heartbeats included, is liveness.
                    last_seen = Instant::now();
                    match self.receive(message).await {
                        Ok(()) => continue,
                        Err(end) => end,
                    }
                }
                _ = tokio::time::sleep_until(age_deadline.unwrap_or(last_seen)),
                    if age_deadline.is_some() =>
                {
                    match self.queue_age_deadline() {
                        Some(d) if d <= Instant::now() => {
                            End::Terminal(TerminalReason::DeliveryOverload)
                        }
                        // The consumer took it meanwhile.
                        _ => continue,
                    }
                }
                _ = tokio::time::sleep_until(last_seen + liveness) => {
                    match self.disconnected(DisconnectReason::LivenessTimeout).await {
                        Ok(()) => continue,
                        Err(end) => end,
                    }
                }
            };
            return (end, Some(socket));
        }
    }

    // Decide a command: apply it whole or not at all, and reply. The reply
    // is acceptance, not broker acknowledgement.
    fn on_command(&mut self, command: Command) {
        let Command::Subscription {
            command,
            reply,
            span,
        } = command;
        let _entered = span.enter();
        let result = self.desired.apply(&command).map(|(revision, changed)| {
            if changed {
                self.unsent.push(revision);
                self.shared.update(|s| s.desired_revision = revision);
            }
            revision
        });
        let kind = status::command_kind(&command);
        match &result {
            Ok(revision) => {
                span.record("revision", revision.0);
                span.record("decision", Decision::Accepted.as_str());
                self.obs.command(kind, Decision::Accepted);
            }
            Err(e) => {
                span.record("decision", Decision::Rejected.as_str());
                span.record("rejection", status::rejection(e));
                self.obs.command(kind, Decision::Rejected);
            }
        }
        if let Some(g) = &self.shared.command_gauges {
            g.set(self.commands.len(), 0, None);
        }
        // The caller may have stopped waiting; the decision stands.
        let _ = reply.send(result);
    }

    // Bring the connection to the desired map: report superseded
    // revisions, write the reconciling requests, then report the latest
    // revision as sent or failed.
    async fn sync(&mut self, socket: &mut Socket) -> Result<(), End> {
        let requests = reconcile(
            self.sent.as_ref().unwrap_or(&BTreeMap::new()),
            self.desired.map(),
        );
        // On a new connection the retained map is restored even when no
        // command is pending.
        if requests.is_empty() && self.unsent.is_empty() {
            return Ok(());
        }
        let latest = self.desired.revision();
        for superseded in std::mem::take(&mut self.unsent) {
            if superseded != latest {
                self.lifecycle(LifecycleKind::Superseded {
                    revision: superseded.0,
                })
                .await?;
            }
        }
        #[cfg(test)]
        let stall = self.faults.stall_sends;
        let write = async {
            #[cfg(test)]
            if stall {
                std::future::pending::<()>().await;
            }
            for r in &requests {
                socket.feed(Message::Text(r.to_string())).await?;
            }
            socket.flush().await
        };
        // One write in progress at a time (`B-TK-13`). Shutdown interrupts
        // it, leaving its delivery unknown; a write that stalls past the
        // liveness timeout is a liveness loss.
        let shared = self.shared.clone();
        self.pending(true);
        let written = tokio::select! {
            biased;
            r = write => r,
            _ = shared.stopped() => {
                self.pending(false);
                self.interrupted = Some(latest);
                self.lifecycle(LifecycleKind::SendFailed { revision: latest.0 })
                    .await?;
                return Err(End::Shutdown);
            }
            _ = tokio::time::sleep(self.limits.reconnect.liveness_timeout()) => {
                self.pending(false);
                self.lifecycle(LifecycleKind::SendFailed { revision: latest.0 })
                    .await?;
                return self.disconnected(DisconnectReason::LivenessTimeout).await;
            }
        };
        self.pending(false);
        #[cfg(test)]
        let written = if self.faults.fail_sends > 0 {
            self.faults.fail_sends -= 1;
            Err(tungstenite::Error::Io(
                std::io::ErrorKind::BrokenPipe.into(),
            ))
        } else {
            written
        };
        match written {
            Ok(()) => {
                self.sent = Some(self.desired.map().clone());
                self.shared.update(|s| s.sent_revision = Some(latest));
                self.lifecycle(LifecycleKind::CommandsSent { revision: latest.0 })
                    .await
            }
            Err(e) => {
                self.lifecycle(LifecycleKind::SendFailed { revision: latest.0 })
                    .await?;
                self.disconnected(classify(&e)).await
            }
        }
    }

    async fn receive(
        &mut self,
        message: Option<Result<Message, tungstenite::Error>>,
    ) -> Result<(), End> {
        let (kind, bytes) = match message {
            Some(Ok(Message::Binary(b))) => {
                self.obs.received(PayloadKindLabel::Binary, b.len());
                // A heartbeat is a one-byte binary message.
                self.shared.traffic.message(b.len() == 1);
                (PayloadKind::Binary, b)
            }
            Some(Ok(Message::Text(t))) => {
                self.obs.received(PayloadKindLabel::Text, t.len());
                self.shared.traffic.message(false);
                (PayloadKind::Text, t.into_bytes())
            }
            Some(Ok(Message::Close(_))) => {
                return self.disconnected(DisconnectReason::RemoteClose).await
            }
            // Ping, pong and frames are transport traffic, not messages.
            Some(Ok(_)) => return Ok(()),
            Some(Err(e)) => return self.disconnected(classify(&e)).await,
            None => return self.disconnected(DisconnectReason::Eof).await,
        };
        let key = self.next_key();
        let observation = RawObservation::new(
            key,
            kind,
            ReceiveTime::now(),
            self.sequencer.elapsed_nanos_now(),
            bytes,
            self.limits.max_payload,
        );
        match observation {
            Ok(o) => {
                self.deliver(TickerEvent::Raw(o)).await?;
                #[cfg(test)]
                if let Some(n) = self.faults.panic_after.as_mut() {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        panic!("injected owner fault");
                    }
                }
                Ok(())
            }
            Err(_) => self.disconnected(DisconnectReason::ProtocolError).await,
        }
    }

    async fn disconnected(&mut self, reason: DisconnectReason) -> Result<(), End> {
        self.state(TickerState::Disconnected);
        self.lifecycle(LifecycleKind::Disconnected { reason })
            .await?;
        Err(End::Lost(reason))
    }

    // Close the socket, deliver the final events and record the outcome.
    async fn finish(&mut self, end: End, socket: Option<Socket>) -> TerminalReason {
        let requested = matches!(end, End::Shutdown);
        // Teardown time runs from the shutdown request, or from now.
        let started = self
            .stop_at
            .map_or_else(Instant::now, |at| at - self.limits.shutdown_deadline);
        self.connected = None;
        let reason = match end {
            End::Shutdown => self.stop(socket).await,
            End::Lost(_) | End::Rejected(_) => unreachable!("the run loop resolves these"),
            End::Terminal(reason) => {
                if let Some(mut socket) = socket {
                    // Best effort, and never past a requested shutdown's
                    // deadline: the ticker is failing anyway.
                    let until = self
                        .stop_at
                        .unwrap_or_else(|| Instant::now() + self.limits.shutdown_deadline);
                    let _ = tokio::time::timeout_at(until, socket.close(None)).await;
                }
                if !matches!(
                    reason,
                    TerminalReason::ReceiverDropped { .. }
                        | TerminalReason::AuthRejected { .. }
                        | TerminalReason::ShutdownDeadlineExpired { .. }
                ) {
                    let _ = self.lifecycle(LifecycleKind::Failed).await;
                }
                reason
            }
        };
        let state = match reason {
            TerminalReason::Shutdown => TickerState::Stopped,
            TerminalReason::AuthRejected { .. } => TickerState::AuthRejected,
            _ => TickerState::Failed,
        };
        let took = started.elapsed();
        let result = status::shutdown_result(&reason);
        tracing::debug_span!(
            "manja.ticker.shutdown",
            reason = if requested { "requested" } else { "terminal" },
            elapsed_ms = took.as_millis().min(u64::MAX as u128) as u64,
            pending_deliveries = self.events.len(),
            result = result.as_str(),
        )
        .in_scope(|| {});
        self.obs.shutdown(result, took);
        self.shared.terminate(state, reason.clone());
        reason
    }

    fn pending(&self, on: bool) {
        self.shared
            .traffic
            .pending_send
            .store(on, Ordering::Relaxed);
        if let Some(g) = &self.pending_gauges {
            g.set(on as usize, 0, None);
        }
    }

    fn record_failure(&self, reason: TerminalReason) {
        let failure = TickerFailure {
            connection_epoch: self.sequencer.epoch(),
            reason,
            at: self.shared.traffic.origin.elapsed(),
        };
        self.shared.update(|s| s.failures.push(failure));
    }

    async fn stop(&mut self, socket: Option<Socket>) -> TerminalReason {
        let stop_at = *self
            .stop_at
            .get_or_insert_with(|| Instant::now() + self.limits.shutdown_deadline);
        self.state(TickerState::Stopping);
        let mut closed = true;
        // After an interrupted write the socket may hold part of a frame: it
        // is dropped, not closed.
        let socket = socket.filter(|_| self.interrupted.is_none());
        if let Some(mut socket) = socket {
            closed = tokio::time::timeout_at(stop_at, async {
                socket.close(None).await?;
                // The handshake is complete when the peer's close frame
                // arrives. Intake has stopped: messages read meanwhile were
                // never accepted and are not delivered.
                loop {
                    match socket.next().await {
                        Some(Ok(Message::Close(_))) | None => return Ok(()),
                        Some(Ok(_)) => {}
                        Some(Err(e)) => return Err(e),
                    }
                }
            })
            .await
            .map(|r| {
                matches!(
                    r,
                    Ok(())
                        | Err(tungstenite::Error::ConnectionClosed
                            | tungstenite::Error::AlreadyClosed)
                )
            })
            .unwrap_or(false);
            let lifecycle = [
                LifecycleKind::Disconnected {
                    reason: DisconnectReason::Shutdown,
                },
                LifecycleKind::Stopped,
            ];
            return self.deliver_final(lifecycle, closed).await;
        }
        self.deliver_final([LifecycleKind::Stopped], closed).await
    }

    async fn deliver_final<const N: usize>(
        &mut self,
        kinds: [LifecycleKind; N],
        closed: bool,
    ) -> TerminalReason {
        let mut remaining = N;
        for kind in kinds {
            match self.lifecycle(kind).await {
                Ok(()) => remaining -= 1,
                Err(End::Terminal(TerminalReason::ShutdownDeadlineExpired { .. })) => {
                    return TerminalReason::ShutdownDeadlineExpired {
                        undelivered: remaining,
                    };
                }
                Err(End::Terminal(r)) => return r,
                Err(_) => unreachable!("delivery ends only in a terminal reason"),
            }
        }
        match (self.interrupted, closed) {
            (Some(r), _) => TerminalReason::SendInterrupted { revision: r.0 },
            (None, true) => TerminalReason::Shutdown,
            (None, false) => TerminalReason::CloseFailed,
        }
    }
}

async fn connect(url: Arc<Secret<String>>, limits: TickerLimits) -> Result<Socket, TerminalReason> {
    let config = WebSocketConfig {
        max_message_size: Some(limits.max_payload),
        max_frame_size: Some(limits.max_payload),
        ..WebSocketConfig::default()
    };
    let attempt = tokio_tungstenite::connect_async_with_config(
        url.expose_secret().as_str(),
        Some(config),
        true,
    );
    match tokio::time::timeout(limits.handshake_timeout, attempt).await {
        Err(_) => Err(TerminalReason::HandshakeTimeout),
        Ok(Ok((socket, _))) => Ok(socket),
        // Only the status is kept: the error may carry the request URL,
        // which holds the access token.
        Ok(Err(tungstenite::Error::Http(response))) => {
            let http_status = response.status().as_u16();
            Err(match http_status {
                401 | 403 => TerminalReason::AuthRejected { http_status },
                _ => TerminalReason::HandshakeRejected { http_status },
            })
        }
        Ok(Err(_)) => Err(TerminalReason::ConnectFailed),
    }
}

fn classify(e: &tungstenite::Error) -> DisconnectReason {
    use tungstenite::error::ProtocolError;
    match e {
        tungstenite::Error::ConnectionClosed
        | tungstenite::Error::AlreadyClosed
        | tungstenite::Error::Protocol(ProtocolError::ResetWithoutClosingHandshake) => {
            DisconnectReason::Eof
        }
        tungstenite::Error::Io(_) | tungstenite::Error::Tls(_) => DisconnectReason::TransportError,
        _ => DisconnectReason::ProtocolError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send_unpin<T: Send + Unpin + 'static>() {}

    #[test]
    fn handle_events_and_guard_have_their_documented_auto_traits() {
        assert_send_sync::<TickerHandle>();
        assert_send_unpin::<TickerEvents>();
        assert_send_unpin::<TaskGuard>();
    }

    #[test]
    fn limits_are_validated() {
        let d = TickerLimits::default();
        assert!(d.clone().with_queue_messages(15).is_err());
        assert!(d.clone().with_queue_messages(16).is_ok());
        assert!(d.clone().with_command_mailbox(0).is_err());
        assert!(d
            .clone()
            .with_handshake_timeout(Duration::from_millis(999))
            .is_err());
        assert!(d.clone().with_max_payload(16 << 20).is_ok());
        assert!(d
            .clone()
            .with_max_payload(2 << 20)
            .unwrap()
            .with_queue_bytes(1 << 20)
            .is_err());
        assert!(d.with_shutdown_deadline(Duration::from_secs(61)).is_err());
    }

    #[test]
    fn spawning_outside_a_runtime_is_an_error() {
        let b = TickerBuilder::new(Credentials::new("k", "t").unwrap());
        assert_eq!(b.spawn().unwrap_err(), TickerSpawnError::NoRuntime);
    }

    // A server that sends `messages` after the handshake, then holds the
    // connection open.
    async fn serve(messages: usize) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                connections.spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                    for _ in 0..messages {
                        ws.send(Message::Binary(vec![0])).await.unwrap();
                    }
                    // Reading also answers a close.
                    while let Some(Ok(_)) = ws.next().await {}
                    std::future::pending::<()>().await;
                });
            }
        });
        (format!("ws://{addr}"), server)
    }

    fn kinds(items: &[Result<TickerEvent, TickerError>]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                Ok(TickerEvent::Lifecycle(l)) => format!("{:?}", l.kind()),
                Ok(TickerEvent::Raw(_)) => "raw".into(),
                Err(e) => format!("err:{:?}", e.reason()),
            })
            .collect()
    }

    #[tokio::test]
    async fn a_failed_restoration_write_never_reports_active() {
        let (url, server) = serve(0).await;
        let one = ReconnectLimits::default().with_attempts(1).unwrap();
        let mut b = TickerBuilder::new(Credentials::new("k", "t").unwrap())
            .url(url)
            .limits(TickerLimits::default().with_reconnect(one));
        b.faults.fail_sends = usize::MAX;
        let (handle, mut events, guard) = b.spawn().unwrap();
        handle
            .subscribe([InstrumentToken::new(1)], Mode::Full)
            .await
            .unwrap();
        let mut items = Vec::new();
        while let Some(i) = events.next().await {
            items.push(i);
        }
        let kinds = kinds(&items);
        assert!(
            kinds.contains(&"SendFailed { revision: 1 }".to_string()),
            "{kinds:?}"
        );
        assert!(!kinds.iter().any(|k| k.starts_with("Active")), "{kinds:?}");
        assert!(
            !kinds.iter().any(|k| k.starts_with("CommandsSent")),
            "{kinds:?}"
        );
        assert_eq!(
            guard.join().await,
            TaskOutcome::Terminal(TerminalReason::ReconnectExhausted {
                attempts: 1,
                last: Box::new(TerminalReason::Disconnected(
                    DisconnectReason::TransportError
                )),
            })
        );
        assert_eq!(handle.status().sent_revision, None);
        server.abort();
    }

    #[tokio::test]
    async fn a_failed_restoration_is_retried_and_only_the_restored_connection_is_active() {
        let (url, server) = serve(0).await;
        let quick = ReconnectLimits::default()
            .with_backoff(Duration::from_millis(50), Duration::from_millis(50))
            .unwrap();
        let mut b = TickerBuilder::new(Credentials::new("k", "t").unwrap())
            .url(url)
            .limits(TickerLimits::default().with_reconnect(quick));
        b.faults.fail_sends = 1;
        let (handle, mut events, _guard) = b.spawn().unwrap();
        handle
            .subscribe([InstrumentToken::new(1)], Mode::Full)
            .await
            .unwrap();
        let mut items = Vec::new();
        while let Some(i) = events.next().await {
            let active = matches!(&i, Ok(TickerEvent::Lifecycle(l)) if matches!(l.kind(), LifecycleKind::Active { .. }));
            items.push(i);
            if active {
                break;
            }
        }
        let epochs: Vec<(u64, String)> = items
            .iter()
            .map(|i| match i {
                Ok(TickerEvent::Lifecycle(l)) => {
                    (l.source().connection_epoch().0, format!("{:?}", l.kind()))
                }
                other => (0, format!("{other:?}")),
            })
            .filter(|(_, k)| !k.starts_with("Backoff") && !k.starts_with("Gap"))
            .collect();
        assert_eq!(
            epochs,
            [
                (1, "ConnectAttempt { attempt: 1 }".to_string()),
                (1, "Connected".into()),
                (1, "SendFailed { revision: 1 }".into()),
                (1, "Disconnected { reason: TransportError }".into()),
                (2, "ConnectAttempt { attempt: 2 }".into()),
                (2, "Connected".into()),
                (2, "CommandsSent { revision: 1 }".into()),
                (2, "Active { revision: 1 }".into()),
            ]
        );
        handle.shutdown().await.unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn shutdown_during_a_stalled_write_is_an_interrupted_send() {
        let (url, server) = serve(0).await;
        let mut b = TickerBuilder::new(Credentials::new("k", "t").unwrap()).url(url);
        b.faults.stall_sends = true;
        let (handle, mut events, guard) = b.spawn().unwrap();
        handle
            .subscribe([InstrumentToken::new(1)], Mode::Full)
            .await
            .unwrap();
        for _ in 0..200 {
            if handle.status().state == TickerState::Restoring {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(handle.status().state, TickerState::Restoring);
        let interrupted = TerminalReason::SendInterrupted { revision: 1 };
        assert_eq!(handle.shutdown().await.unwrap_err().reason(), &interrupted);
        let mut items = Vec::new();
        while let Some(i) = events.next().await {
            items.push(i);
        }
        let kinds = kinds(&items);
        assert!(
            kinds.contains(&"SendFailed { revision: 1 }".to_string()),
            "{kinds:?}"
        );
        assert!(
            !kinds.iter().any(|k| k.starts_with("CommandsSent")),
            "{kinds:?}"
        );
        assert_eq!(
            kinds.last().unwrap(),
            &format!("err:{interrupted:?}"),
            "not a clean end"
        );
        assert_eq!(guard.join().await, TaskOutcome::Terminal(interrupted));
        server.abort();
    }

    #[tokio::test]
    async fn a_panicking_owner_is_reported_once_and_supervised() {
        let (url, server) = serve(3).await;
        let mut b = TickerBuilder::new(Credentials::new("k", "t").unwrap()).url(url);
        b.faults.panic_after = Some(2);
        let (handle, mut events, guard) = b.spawn().unwrap();
        let mut raw = 0;
        let mut errors = Vec::new();
        while let Some(item) = events.next().await {
            match item {
                Ok(TickerEvent::Raw(_)) => raw += 1,
                Ok(_) => {}
                Err(e) => errors.push(e),
            }
        }
        assert_eq!(raw, 2, "events delivered before the fault are kept");
        assert_eq!(errors, [TickerError(TerminalReason::Panicked)]);
        assert!(events.next().await.is_none());
        assert_eq!(guard.join().await, TaskOutcome::Panicked);
        let status = handle.status();
        assert_eq!(status.state, TickerState::Failed);
        assert_eq!(status.terminal, Some(TerminalReason::Panicked));
        server.abort();
    }
}
