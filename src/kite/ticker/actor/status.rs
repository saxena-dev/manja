//! The queryable ticker status snapshot and ticker instrumentation.
//!
//! # Status
//!
//! [`TickerStatus`] is assembled when it is read: the owner's state, epoch,
//! revisions, failure history and terminal reason, plus ages and queue
//! measures computed at that moment. Ages therefore keep advancing when
//! traffic stops. Reading it never waits on the data queue. The
//! `snapshot_revision` changes on every owner state change, not on every
//! message.
//!
//! [`TickerHandle::changed`](crate::kite::ticker::actor::owner::TickerHandle::changed)
//! is the notification path. It coalesces: a caller that falls behind
//! receives the latest snapshot, and the difference in `snapshot_revision`
//! is the number of changes it did not see. It is best-effort by design and
//! never an authoritative record of events; the primary stream is. Slow or
//! absent status readers affect neither the owner, nor delivery, nor the
//! shutdown deadline.
//!
//! # Instrumentation
//!
//! With an [`Observability`] scope that records, the owner reports:
//!
//! | Fact | Instrument |
//! |---|---|
//! | every handshake attempt, its result and duration | `manja_ticker_connection_attempts_total`, `manja_ticker_connect_duration_seconds` |
//! | established sockets | `manja_ticker_connections_active` |
//! | reconnect attempts that actually start | `manja_ticker_reconnects_total{reason}` |
//! | command decisions | `manja_ticker_commands_total{command, decision}` |
//! | restoration of the desired map on a connection | `manja_ticker_restore_duration_seconds{result}` |
//! | complete binary and text messages, heartbeats included | `manja_ticker_received_messages_total`, `manja_ticker_received_bytes_total` |
//! | SDK queues and the pending send | `manja_sdk_queue_messages`, `manja_sdk_queue_retained_bytes`, `manja_sdk_queue_oldest_age_seconds` |
//! | teardown | `manja_ticker_shutdown_duration_seconds{result}` |
//!
//! Messages are counted at the application-message boundary: one per
//! complete WebSocket message, never per decoded packet or per frame.
//! Labels are closed domains only; the feed identity and the epoch appear
//! as span fields, never as labels, and no token or broker text appears in
//! either. The spans are `manja.ticker.connection`, `manja.ticker.restore`,
//! `manja.ticker.command` (created in the caller's context and carried
//! through the mailbox) and `manja.ticker.shutdown`. Gauge contributions
//! are removed when their owner goes away.
//!
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::time::Instant;

use crate::kite::envelope::{ConnectionEpoch, DisconnectReason};
use crate::kite::obs::diagnostics::{FailureHistory, DEFAULT_HISTORY};
use crate::kite::obs::handle::{GaugeGuard, Labels, Observability};
use crate::kite::obs::schema::{
    CommandKind, ConnectionResult, Decision, Instrument, PayloadKindLabel, QueueRole,
    ReconnectReason, RestoreResult, ShutdownResult,
};
use crate::kite::ticker::actor::owner::{TerminalReason, TickerState};
use crate::kite::ticker::actor::subscriptions::{Revision, SubscriptionCommand, SubscriptionError};

/// SDK queue measures at the time of the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueStatus {
    /// Events in the primary queue.
    pub messages: usize,
    /// Bytes charged to them, backing allocations included.
    pub retained_bytes: usize,
    /// Age of the oldest queued event.
    pub oldest_age: Option<Duration>,
    /// Commands waiting in the mailbox.
    pub commands: usize,
    /// Whether a subscription write is in progress.
    pub pending_send: bool,
}

/// One recorded connection failure.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TickerFailure {
    /// Epoch of the attempt or connection.
    pub connection_epoch: ConnectionEpoch,
    /// What happened.
    pub reason: TerminalReason,
    /// Time since the ticker started.
    pub at: Duration,
}

/// A snapshot of the owner's state, read without touching the data queue.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TickerStatus {
    /// Connection state. `Active` is not quote freshness.
    pub state: TickerState,
    /// Epoch of the current or last connection attempt; 0 before the first.
    pub connection_epoch: ConnectionEpoch,
    /// Revision of the desired subscription map.
    pub desired_revision: Revision,
    /// The last revision written to a connection, if any.
    pub sent_revision: Option<Revision>,
    /// Time since the last complete message of any kind.
    pub last_message_age: Option<Duration>,
    /// Time since the last heartbeat (a one-byte binary message).
    pub last_heartbeat_age: Option<Duration>,
    /// SDK queue measures.
    pub queue: QueueStatus,
    /// The most recent connection failures, oldest first (`B-DIAG-01`).
    pub last_failures: Vec<TickerFailure>,
    /// The sticky terminal reason, once the ticker has ended.
    pub terminal: Option<TerminalReason>,
    /// Incremented on every owner state change.
    pub snapshot_revision: u64,
    /// When the snapshot was taken, as time since the ticker started.
    pub taken_at: Duration,
}

// The owner-maintained part of the status, published on every state
// change.
#[derive(Clone)]
pub(crate) struct StatusCore {
    pub(crate) state: TickerState,
    pub(crate) connection_epoch: ConnectionEpoch,
    pub(crate) desired_revision: Revision,
    pub(crate) sent_revision: Option<Revision>,
    pub(crate) failures: FailureHistory<TickerFailure>,
    pub(crate) terminal: Option<TerminalReason>,
    pub(crate) snapshot_revision: u64,
}

impl StatusCore {
    pub(crate) fn new() -> Self {
        Self {
            state: TickerState::Disconnected,
            connection_epoch: ConnectionEpoch(0),
            desired_revision: Revision(0),
            sent_revision: None,
            failures: FailureHistory::new(DEFAULT_HISTORY),
            terminal: None,
            snapshot_revision: 0,
        }
    }
}

// Traffic facts updated per message without publishing a new snapshot.
pub(crate) struct Traffic {
    pub(crate) origin: Instant,
    // Nanoseconds since `origin` plus one; 0 means never.
    last_message: AtomicU64,
    last_heartbeat: AtomicU64,
    pub(crate) pending_send: AtomicBool,
}

impl Traffic {
    pub(crate) fn new() -> Self {
        Self {
            origin: Instant::now(),
            last_message: AtomicU64::new(0),
            last_heartbeat: AtomicU64::new(0),
            pending_send: AtomicBool::new(false),
        }
    }

    fn stamp(&self) -> u64 {
        (self.origin.elapsed().as_nanos().min(u64::MAX as u128 - 1) as u64) + 1
    }

    pub(crate) fn message(&self, heartbeat: bool) {
        let now = self.stamp();
        self.last_message.store(now, Ordering::Relaxed);
        if heartbeat {
            self.last_heartbeat.store(now, Ordering::Relaxed);
        }
    }

    fn age(&self, stamp: &AtomicU64, now: Duration) -> Option<Duration> {
        match stamp.load(Ordering::Relaxed) {
            0 => None,
            s => Some(now.saturating_sub(Duration::from_nanos(s - 1))),
        }
    }

    pub(crate) fn snapshot(&self, core: &StatusCore, queue: QueueStatus) -> TickerStatus {
        let now = self.origin.elapsed();
        TickerStatus {
            state: core.state,
            connection_epoch: core.connection_epoch,
            desired_revision: core.desired_revision,
            sent_revision: core.sent_revision,
            last_message_age: self.age(&self.last_message, now),
            last_heartbeat_age: self.age(&self.last_heartbeat, now),
            queue,
            last_failures: core.failures.entries(),
            terminal: core.terminal.clone(),
            snapshot_revision: core.snapshot_revision,
            taken_at: now,
        }
    }
}

/// The ticker's recording helpers over one observability scope.
#[derive(Clone)]
pub(crate) struct TickerObs {
    pub(crate) obs: Observability,
    pub(crate) feed_id: String,
}

impl TickerObs {
    pub(crate) fn connection_attempt(&self, result: ConnectionResult, took: Duration) {
        self.obs.counter(
            Labels::connection(Instrument::TickerConnectionAttemptsTotal, result),
            1,
        );
        self.obs.histogram(
            Labels::connection(Instrument::TickerConnectDuration, result),
            took.as_secs_f64(),
        );
    }

    /// A contribution of one established socket, removed on drop.
    pub(crate) fn connected(&self) -> GaugeGuard {
        let g = self
            .obs
            .gauge(Labels::none(Instrument::TickerConnectionsActive));
        g.set(1);
        g
    }

    pub(crate) fn reconnect(&self, reason: ReconnectReason) {
        self.obs.counter(Labels::reconnect(reason), 1);
    }

    pub(crate) fn command(&self, kind: CommandKind, decision: Decision) {
        self.obs.counter(Labels::command(kind, decision), 1);
    }

    pub(crate) fn restore(&self, result: RestoreResult, took: Duration) {
        self.obs
            .histogram(Labels::restore(result), took.as_secs_f64());
    }

    pub(crate) fn received(&self, kind: PayloadKindLabel, bytes: usize) {
        self.obs.counter(
            Labels::payload(Instrument::TickerReceivedMessagesTotal, kind),
            1,
        );
        self.obs.counter(
            Labels::payload(Instrument::TickerReceivedBytesTotal, kind),
            bytes as u64,
        );
    }

    pub(crate) fn shutdown(&self, result: ShutdownResult, took: Duration) {
        self.obs
            .histogram(Labels::shutdown(result), took.as_secs_f64());
    }

    /// Gauges for one SDK queue, or none when nothing records.
    pub(crate) fn queue_gauges(&self, role: QueueRole) -> Option<QueueGauges> {
        self.obs.is_recording().then(|| QueueGauges {
            messages: self
                .obs
                .gauge(Labels::queue(Instrument::SdkQueueMessages, role)),
            bytes: self
                .obs
                .gauge(Labels::queue(Instrument::SdkQueueRetainedBytes, role)),
            age: self.obs.age(role),
        })
    }
}

/// Gauge contributions of one SDK queue.
pub(crate) struct QueueGauges {
    messages: GaugeGuard,
    bytes: GaugeGuard,
    age: crate::kite::obs::handle::AgeGuard,
}

impl QueueGauges {
    pub(crate) fn set(&self, messages: usize, bytes: usize, oldest: Option<Instant>) {
        self.messages.set(messages as i64);
        self.bytes.set(bytes as i64);
        self.age.set_oldest(oldest.map(Instant::into_std));
    }
}

/// The connection-attempt result label of a failed attempt.
pub(crate) fn connection_result(reason: &TerminalReason) -> ConnectionResult {
    match reason {
        TerminalReason::AuthRejected { .. } => ConnectionResult::AuthRejected,
        TerminalReason::HandshakeRejected { .. } => ConnectionResult::HttpStatus,
        TerminalReason::HandshakeTimeout => ConnectionResult::Timeout,
        _ => ConnectionResult::TransportError,
    }
}

/// The reconnect-reason label for the failure that caused a reconnect.
pub(crate) fn reconnect_reason(failure: &TerminalReason) -> ReconnectReason {
    match failure {
        TerminalReason::Disconnected(DisconnectReason::Eof) => ReconnectReason::Eof,
        TerminalReason::Disconnected(DisconnectReason::RemoteClose) => ReconnectReason::RemoteClose,
        TerminalReason::Disconnected(DisconnectReason::LivenessTimeout) => {
            ReconnectReason::LivenessTimeout
        }
        TerminalReason::Disconnected(DisconnectReason::ProtocolError) => {
            ReconnectReason::ProtocolError
        }
        _ => ReconnectReason::TransportError,
    }
}

/// The command label of a subscription command.
pub(crate) fn command_kind(command: &SubscriptionCommand) -> CommandKind {
    match command {
        SubscriptionCommand::Subscribe { .. } => CommandKind::Subscribe,
        SubscriptionCommand::Unsubscribe { .. } => CommandKind::Unsubscribe,
        SubscriptionCommand::SetMode { .. } => CommandKind::SetMode,
        SubscriptionCommand::Replace { .. } => CommandKind::Replace,
    }
}

/// The shutdown-result label of a terminal reason.
pub(crate) fn shutdown_result(reason: &TerminalReason) -> ShutdownResult {
    match reason {
        TerminalReason::Shutdown => ShutdownResult::Clean,
        TerminalReason::ShutdownDeadlineExpired { .. } => ShutdownResult::DeadlineExpired,
        TerminalReason::Panicked => ShutdownResult::Panicked,
        _ => ShutdownResult::Failed,
    }
}

/// A bounded name for a command rejection, for span fields.
pub(crate) fn rejection(e: &SubscriptionError) -> &'static str {
    match e {
        SubscriptionError::Empty => "empty",
        SubscriptionError::ConflictingModes { .. } => "conflicting_modes",
        SubscriptionError::NotDesired { .. } => "not_desired",
        SubscriptionError::CapacityExceeded { .. } => "capacity_exceeded",
    }
}
