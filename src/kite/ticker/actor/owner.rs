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
//! [`TerminalReason::DeliveryOverload`]; nothing is silently dropped. Status
//! and shutdown never need room in the queue.
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
//! This owner makes one connection attempt and does not reconnect; that
//! layer comes later.
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
use tokio::sync::{mpsc, oneshot, watch, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::{self, protocol::WebSocketConfig, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::kite::connect::credentials::Credentials;
use crate::kite::envelope::{
    ConnectionEpoch, DisconnectReason, LifecycleEvent, LifecycleKind, PayloadKind, RawObservation,
    ReceiveTime, SourceIdentity, SourceSequencer, MAX_PAYLOAD_BYTES_LIMIT,
};
use crate::kite::obs::Observability;
use crate::kite::protocol::InstrumentToken;
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
    command_mailbox: usize,
    max_payload: usize,
    max_instruments: usize,
    shutdown_deadline: Duration,
}

impl Default for TickerLimits {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(10),
            queue_messages: 4096,
            queue_bytes: 64 << 20,
            delivery_wait: Duration::from_secs(1),
            command_mailbox: 64,
            max_payload: 1 << 20,
            max_instruments: MAX_INSTRUMENTS_PER_CONNECTION,
            shutdown_deadline: Duration::from_secs(5),
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

    /// Primary delivery wait (`B-TK-08`, 10 ms to 30 s).
    pub fn with_delivery_wait(mut self, d: Duration) -> Result<Self, TickerLimitError> {
        self.delivery_wait = within(
            "B-TK-08",
            d,
            Duration::from_millis(10),
            Duration::from_secs(30),
        )?;
        Ok(self)
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

    fn check(self) -> Result<Self, TickerLimitError> {
        if self.queue_bytes < self.max_payload {
            return Err(TickerLimitError("B-TK-06 >= B-TK-10"));
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
    ReceiverDropped,
    /// Every [`TickerHandle`] was dropped.
    HandlesDropped,
    /// The close handshake failed during shutdown.
    CloseFailed,
    /// Shutdown did not deliver every event within `B-TK-12`.
    ShutdownDeadlineExpired {
        /// Events the owner had accepted but could not deliver.
        undelivered: usize,
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
            Self::ReceiverDropped => f.write_str("the primary receiver was dropped"),
            Self::HandlesDropped => f.write_str("every ticker handle was dropped"),
            Self::CloseFailed => f.write_str("the close handshake failed"),
            Self::ShutdownDeadlineExpired { undelivered } => {
                write!(f, "shutdown expired with {undelivered} events undelivered")
            }
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

/// A snapshot of the owner's state, read without touching the data queue.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TickerStatus {
    /// Connection state.
    pub state: TickerState,
    /// Epoch of the current or last connection attempt; 0 before the first.
    pub connection_epoch: ConnectionEpoch,
    /// Revision of the desired subscription map.
    pub desired_revision: Revision,
    /// The last revision written to a connection, if any.
    pub sent_revision: Option<Revision>,
    /// The sticky terminal reason, once the ticker has ended.
    pub terminal: Option<TerminalReason>,
    /// Incremented on every change, so a stale snapshot is detectable.
    pub snapshot_revision: u64,
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
    status: watch::Sender<TickerStatus>,
    stop: Notify,
    stop_requested: AtomicBool,
}

impl Shared {
    fn update(&self, f: impl FnOnce(&mut TickerStatus)) {
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

struct Queued {
    event: TickerEvent,
    _charge: OwnedSemaphorePermit,
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
        let (status, status_rx) = watch::channel(TickerStatus {
            state: TickerState::Disconnected,
            connection_epoch: ConnectionEpoch(0),
            desired_revision: Revision(0),
            sent_revision: None,
            terminal: None,
            snapshot_revision: 0,
        });
        let shared = Arc::new(Shared {
            status,
            stop: Notify::new(),
            stop_requested: AtomicBool::new(false),
        });
        let (commands_tx, commands) = mpsc::channel(self.limits.command_mailbox);
        let (events_tx, events_rx) = mpsc::channel(self.limits.queue_messages);
        let owner = Owner {
            url: Arc::new(url),
            desired: DesiredSubscriptions::new(self.limits.max_instruments),
            sent: None,
            unsent: Vec::new(),
            limits: self.limits.clone(),
            sequencer: SourceSequencer::new(identity),
            shared: shared.clone(),
            commands,
            events: events_tx.clone(),
            bytes: Arc::new(Semaphore::new(self.limits.queue_bytes)),
            stop_at: None,
            _observability: self.observability,
            #[cfg(test)]
            faults: self.faults,
        };
        let supervisor = shared.clone();
        let task = runtime.spawn(async move {
            // Held until the terminal reason is recorded, so the receiver
            // never sees the end of the queue before the reason.
            let _keep_open = events_tx;
            let reason = match AssertUnwindSafe(owner.run()).catch_unwind().await {
                Ok(reason) => reason,
                Err(_) => {
                    supervisor.terminate(TickerState::Failed, TerminalReason::Panicked);
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
    status: watch::Receiver<TickerStatus>,
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
        self.status.borrow().clone()
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
        self.commands
            .try_send(Command::Subscription { command, reply })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => CommandError::MailboxFull,
                mpsc::error::TrySendError::Closed(_) => self.terminated(),
            })?;
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
    rx: mpsc::Receiver<Queued>,
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
            // The byte charge is released here, as ownership passes on.
            Poll::Ready(Some(q)) => Poll::Ready(Some(Ok(q.event))),
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
    /// Fail every subscription write.
    pub(crate) fail_sends: bool,
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
    events: mpsc::Sender<Queued>,
    bytes: Arc<Semaphore>,
    // Set when shutdown was requested: every remaining delivery must
    // finish by then.
    stop_at: Option<Instant>,
    _observability: Observability,
    #[cfg(test)]
    faults: Faults,
}

// Why the connection phase ended.
enum End {
    Shutdown,
    Terminal(TerminalReason),
}

impl Owner {
    async fn run(mut self) -> TerminalReason {
        let (end, socket) = self.connection().await;
        self.finish(end, socket).await
    }

    fn state(&self, state: TickerState) {
        let epoch = self.sequencer.epoch();
        self.shared.update(|s| {
            s.state = state;
            s.connection_epoch = epoch;
        });
    }

    fn lifecycle_event(&mut self, kind: LifecycleKind) -> TickerEvent {
        let key = self.sequencer.next_key();
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

    fn charge(event: &TickerEvent) -> usize {
        match event {
            TickerEvent::Raw(r) => r.payload().retained_bytes(),
            TickerEvent::Lifecycle(_) => 0,
        }
    }

    // Queue one event, waiting for room: up to `B-TK-08` normally, and up
    // to the shutdown deadline once shutdown was requested.
    async fn deliver(&mut self, event: TickerEvent) -> Result<(), End> {
        let charge = Self::charge(&event).min(self.limits.queue_bytes) as u32;
        let (events, bytes, shared) =
            (self.events.clone(), self.bytes.clone(), self.shared.clone());
        let room = async move {
            let slot = events.reserve_owned().await.map_err(|_| ())?;
            let bytes = bytes.acquire_many_owned(charge).await.map_err(|_| ())?;
            Ok::<_, ()>((slot, bytes))
        };
        tokio::pin!(room);
        let normal_until = Instant::now() + self.limits.delivery_wait;
        loop {
            let until = self.stop_at.unwrap_or(normal_until);
            tokio::select! {
                biased;
                r = &mut room => {
                    return match r {
                        Ok((slot, bytes)) => {
                            slot.send(Queued { event, _charge: bytes });
                            Ok(())
                        }
                        Err(()) => Err(End::Terminal(TerminalReason::ReceiverDropped)),
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

    async fn connection(&mut self) -> (End, Option<Socket>) {
        self.sequencer.begin_epoch();
        self.state(TickerState::Connecting);
        if let Err(end) = self
            .lifecycle(LifecycleKind::ConnectAttempt { attempt: 1 })
            .await
        {
            return (end, None);
        }
        let shared = self.shared.clone();
        let attempt = connect(self.url.clone(), self.limits.clone());
        tokio::pin!(attempt);
        // Commands are accepted while connecting; restoration sends them.
        let connected = loop {
            tokio::select! {
                biased;
                _ = shared.stopped() => return (End::Shutdown, None),
                _ = self.events.closed() => {
                    return (End::Terminal(TerminalReason::ReceiverDropped), None)
                }
                command = self.commands.recv() => match command {
                    None => return (End::Terminal(TerminalReason::HandlesDropped), None),
                    Some(c) => self.on_command(c),
                },
                r = &mut attempt => break r,
            }
        };
        let mut socket = match connected {
            Ok(socket) => socket,
            Err(reason) => {
                if let TerminalReason::AuthRejected { http_status } = reason {
                    // Terminal: the ticker never retries rejected
                    // credentials.
                    let _ = self
                        .lifecycle(LifecycleKind::AuthRejected { http_status })
                        .await;
                }
                return (End::Terminal(reason), None);
            }
        };
        if let Err(end) = self.lifecycle(LifecycleKind::Connected).await {
            return (end, Some(socket));
        }
        // Restore the desired map before reporting `Active`.
        self.state(TickerState::Restoring);
        self.sent = Some(BTreeMap::new());
        if let Err(end) = self.sync(&mut socket).await {
            return (end, Some(socket));
        }
        let revision = self.desired.revision().0;
        if let Err(end) = self.lifecycle(LifecycleKind::Active { revision }).await {
            return (end, Some(socket));
        }
        self.state(TickerState::Active);
        loop {
            let end = tokio::select! {
                biased;
                _ = shared.stopped() => End::Shutdown,
                _ = self.events.closed() => End::Terminal(TerminalReason::ReceiverDropped),
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
                message = socket.next() => match self.receive(message).await {
                    Ok(()) => continue,
                    Err(end) => end,
                },
            };
            return (end, Some(socket));
        }
    }

    // Decide a command: apply it whole or not at all, and reply. The reply
    // is acceptance, not broker acknowledgement.
    fn on_command(&mut self, command: Command) {
        let Command::Subscription { command, reply } = command;
        let result = self.desired.apply(&command).map(|(revision, changed)| {
            if changed {
                self.unsent.push(revision);
                self.shared.update(|s| s.desired_revision = revision);
            }
            revision
        });
        // The caller may have stopped waiting; the decision stands.
        let _ = reply.send(result);
    }

    // Bring the connection to the desired map: report superseded
    // revisions, write the reconciling requests, then report the latest
    // revision as sent or failed.
    async fn sync(&mut self, socket: &mut Socket) -> Result<(), End> {
        let Some(latest) = self.unsent.last().copied() else {
            return Ok(());
        };
        for superseded in std::mem::take(&mut self.unsent) {
            if superseded != latest {
                self.lifecycle(LifecycleKind::Superseded {
                    revision: superseded.0,
                })
                .await?;
            }
        }
        let requests = reconcile(
            self.sent.as_ref().unwrap_or(&BTreeMap::new()),
            self.desired.map(),
        );
        let written = async {
            for r in &requests {
                socket.feed(Message::Text(r.to_string())).await?;
            }
            socket.flush().await
        }
        .await;
        #[cfg(test)]
        let written = if self.faults.fail_sends {
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
            Some(Ok(Message::Binary(b))) => (PayloadKind::Binary, b),
            Some(Ok(Message::Text(t))) => (PayloadKind::Text, t.into_bytes()),
            Some(Ok(Message::Close(_))) => {
                return self.disconnected(DisconnectReason::RemoteClose).await
            }
            // Ping, pong and frames are transport traffic, not messages.
            Some(Ok(_)) => return Ok(()),
            Some(Err(e)) => return self.disconnected(classify(&e)).await,
            None => return self.disconnected(DisconnectReason::Eof).await,
        };
        let key = self.sequencer.next_key();
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
        Err(End::Terminal(TerminalReason::Disconnected(reason)))
    }

    // Close the socket, deliver the final events and record the outcome.
    async fn finish(&mut self, end: End, socket: Option<Socket>) -> TerminalReason {
        let reason = match end {
            End::Shutdown => self.stop(socket).await,
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
                    TerminalReason::ReceiverDropped
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
        self.shared.terminate(state, reason.clone());
        reason
    }

    async fn stop(&mut self, socket: Option<Socket>) -> TerminalReason {
        let stop_at = *self
            .stop_at
            .get_or_insert_with(|| Instant::now() + self.limits.shutdown_deadline);
        self.state(TickerState::Stopping);
        let mut closed = true;
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
                Err(End::Shutdown) => unreachable!("delivery never ends in shutdown"),
            }
        }
        if closed {
            TerminalReason::Shutdown
        } else {
            TerminalReason::CloseFailed
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
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            for _ in 0..messages {
                ws.send(Message::Binary(vec![0])).await.unwrap();
            }
            std::future::pending::<()>().await;
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
        let mut b = TickerBuilder::new(Credentials::new("k", "t").unwrap()).url(url);
        b.faults.fail_sends = true;
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
            TaskOutcome::Terminal(TerminalReason::Disconnected(
                DisconnectReason::TransportError
            ))
        );
        assert_eq!(handle.status().sent_revision, None);
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
