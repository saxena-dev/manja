//! Raw-observation and source-lifecycle envelope types.
//!
//! A [`RawObservation`] is one application message exactly as the WebSocket
//! library delivered it: binary or text bytes, unchanged. It is not a TCP,
//! TLS or WebSocket-frame capture, and it says nothing about broker events
//! that never reached this process. A [`LifecycleEvent`] is a fact about the
//! source itself, such as a connection attempt or a disconnect.
//!
//! Both carry a [`SourceKey`], `(producer_id, run_id, feed_id,
//! connection_epoch, ingress_sequence)`. Within one run the source owner
//! assigns a fresh [`ConnectionEpoch`] to every connection attempt, failed
//! ones included, and raw and lifecycle events share one ingress sequence per
//! epoch. That is a local source order only: it is neither exchange order nor
//! an application's causal order, and equal payloads at different keys are
//! distinct observations.
//!
//! Every portable field is plain data. No `Instant`, socket, task handle or
//! credential appears in an envelope, and nothing here requires an async
//! runtime, a storage format or a decoder. Consumers define their own
//! persistent record schemas; [`ENVELOPE_VERSION`] versions only this
//! in-memory/portable form, independently of decoder and archive versions.
//!
//! ```
//! use manja::kite::envelope::{
//!     PayloadKind, RawObservation, ReceiveTime, SourceIdentity, SourceSequencer,
//!     DEFAULT_MAX_PAYLOAD_BYTES,
//! };
//!
//! // Convenience identity: valid without any capture configuration.
//! let mut seq = SourceSequencer::new(SourceIdentity::generate());
//! let epoch = seq.begin_epoch();
//! let key = seq.next_key();
//! let obs = RawObservation::new(
//!     key,
//!     PayloadKind::Binary,
//!     ReceiveTime::from_unix_nanos(1_769_019_551_354_728_000),
//!     seq.elapsed_nanos_now(),
//!     vec![0x01], // the one-byte heartbeat is an ordinary binary observation
//!     DEFAULT_MAX_PAYLOAD_BYTES,
//! )
//! .unwrap();
//! assert_eq!(obs.source().connection_epoch(), epoch);
//! assert_eq!(obs.payload().as_bytes(), &[0x01]);
//! ```
//!
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A `(major, minor)` envelope version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EnvelopeVersion {
    /// Incremented when a field is removed, renamed or changes type, unit or
    /// meaning. A reader rejects an unsupported major version.
    pub major: u16,
    /// Incremented for additive optional fields only.
    pub minor: u16,
}

/// The envelope version this build produces and reads: `1.0`.
pub const ENVELOPE_VERSION: EnvelopeVersion = EnvelopeVersion { major: 1, minor: 0 };

impl EnvelopeVersion {
    /// Accept any minor version of the supported major; reject other majors.
    pub fn check_supported(self) -> Result<(), EnvelopeError> {
        if self.major == ENVELOPE_VERSION.major {
            Ok(())
        } else {
            Err(EnvelopeError::UnsupportedVersion(self))
        }
    }
}

impl fmt::Display for EnvelopeVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Default maximum payload length, 1 MiB (`B-TK-10`).
///
/// It covers the largest full-mode message for a full connection,
/// `2 + 3000 × (2 + 184) = 558 002` bytes.
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 1 << 20;
/// Largest configurable payload bound, 16 MiB (`B-TK-10`).
pub const MAX_PAYLOAD_BYTES_LIMIT: usize = 16 << 20;
/// Maximum length of a producer or feed identifier, in bytes.
pub const MAX_ID_BYTES: usize = 64;

/// Why an envelope could not be built or read.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvelopeError {
    /// The envelope's major version is not supported by this build.
    UnsupportedVersion(EnvelopeVersion),
    /// A payload exceeded the configured maximum.
    PayloadTooLarge {
        /// Payload length in bytes.
        len: usize,
        /// Configured maximum in bytes.
        max: usize,
    },
    /// A producer or feed identifier was empty, longer than
    /// [`MAX_ID_BYTES`], or not printable ASCII.
    InvalidIdentifier(&'static str),
    /// A text payload was not valid UTF-8.
    InvalidText,
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => write!(f, "unsupported envelope version {v}"),
            Self::PayloadTooLarge { len, max } => {
                write!(f, "payload of {len} bytes exceeds the {max}-byte bound")
            }
            Self::InvalidIdentifier(which) => write!(f, "invalid {which}"),
            Self::InvalidText => f.write_str("text payload is not valid UTF-8"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

fn check_id(which: &'static str, value: &str) -> Result<(), EnvelopeError> {
    let ok = !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value.bytes().all(|b| b.is_ascii_graphic());
    if ok {
        Ok(())
    } else {
        Err(EnvelopeError::InvalidIdentifier(which))
    }
}

/// A 128-bit run identifier, unique per process incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RunId(u128);

impl RunId {
    /// Wrap an explicit run identifier. The caller guarantees that it is
    /// never reused by another run of the same producer.
    pub fn from_u128(value: u128) -> Self {
        Self(value)
    }

    /// A collision-resistant identifier for a new run, mixing the wall clock,
    /// the process ID, a process-local counter and the standard library's
    /// per-process random hash seed.
    pub fn generate() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let mut halves = [0u64; 2];
        for (i, half) in halves.iter_mut().enumerate() {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(now);
            h.write_u32(std::process::id());
            h.write_u64(count);
            h.write_usize(i);
            *half = h.finish();
        }
        Self(((halves[0] as u128) << 64) | halves[1] as u128)
    }

    /// The raw identifier.
    pub fn as_u128(self) -> u128 {
        self.0
    }
}

/// Who produced the observations, which run of it, and which logical feed.
///
/// The feed identifier distinguishes independently subscribed connections; it
/// must never be derived from credentials. Identifiers are printable ASCII of
/// at most [`MAX_ID_BYTES`] bytes.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SourceIdentity(Arc<IdentityParts>);

#[derive(PartialEq, Eq, Hash)]
struct IdentityParts {
    producer_id: String,
    run_id: RunId,
    feed_id: String,
}

impl SourceIdentity {
    /// Explicit identity for advanced use. The caller guarantees that
    /// `(producer_id, run_id, feed_id)` is not reused by another source; a
    /// reused identity would make distinct observations share source keys.
    pub fn new(
        producer_id: impl Into<String>,
        run_id: RunId,
        feed_id: impl Into<String>,
    ) -> Result<Self, EnvelopeError> {
        let producer_id = producer_id.into();
        let feed_id = feed_id.into();
        check_id("producer_id", &producer_id)?;
        check_id("feed_id", &feed_id)?;
        Ok(Self(Arc::new(IdentityParts {
            producer_id,
            run_id,
            feed_id,
        })))
    }

    /// Convenience identity: producer `manja`, a freshly generated run ID and
    /// feed `default`. Valid without any capture configuration.
    pub fn generate() -> Self {
        Self::new("manja", RunId::generate(), "default").expect("static identifiers are valid")
    }

    /// Producer identifier.
    pub fn producer_id(&self) -> &str {
        &self.0.producer_id
    }

    /// Run identifier.
    pub fn run_id(&self) -> RunId {
        self.0.run_id
    }

    /// Logical feed identifier.
    pub fn feed_id(&self) -> &str {
        &self.0.feed_id
    }
}

impl fmt::Debug for SourceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceIdentity")
            .field("producer_id", &self.0.producer_id)
            .field("run_id", &format_args!("{:032x}", self.0.run_id.0))
            .field("feed_id", &self.0.feed_id)
            .finish()
    }
}

/// A connection epoch: assigned to each connection attempt, failed attempts
/// included, and never reused within a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConnectionEpoch(pub u64);

/// The full source key of an observation or lifecycle event.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceKey {
    identity: SourceIdentity,
    connection_epoch: ConnectionEpoch,
    ingress_sequence: u64,
}

impl SourceKey {
    /// Build a key from its parts. Normally keys come from a
    /// [`SourceSequencer`], which guarantees epoch and sequence uniqueness.
    pub fn new(
        identity: SourceIdentity,
        connection_epoch: ConnectionEpoch,
        ingress_sequence: u64,
    ) -> Self {
        Self {
            identity,
            connection_epoch,
            ingress_sequence,
        }
    }

    /// Producer, run and feed identity.
    pub fn identity(&self) -> &SourceIdentity {
        &self.identity
    }

    /// Connection epoch.
    pub fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    /// Ingress sequence within the epoch, starting at 0.
    pub fn ingress_sequence(&self) -> u64 {
        self.ingress_sequence
    }
}

impl fmt::Display for SourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{:032x}/{}/{}/{}",
            self.identity.producer_id(),
            self.identity.run_id().as_u128(),
            self.identity.feed_id(),
            self.connection_epoch.0,
            self.ingress_sequence
        )
    }
}

#[derive(Serialize, Deserialize)]
struct SourceKeyRepr {
    producer_id: String,
    run_id: RunId,
    feed_id: String,
    connection_epoch: ConnectionEpoch,
    ingress_sequence: u64,
}

impl Serialize for SourceKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        SourceKeyRepr {
            producer_id: self.identity.producer_id().to_string(),
            run_id: self.identity.run_id(),
            feed_id: self.identity.feed_id().to_string(),
            connection_epoch: self.connection_epoch,
            ingress_sequence: self.ingress_sequence,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for SourceKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = SourceKeyRepr::deserialize(d)?;
        let identity = SourceIdentity::new(r.producer_id, r.run_id, r.feed_id)
            .map_err(serde::de::Error::custom)?;
        Ok(Self::new(identity, r.connection_epoch, r.ingress_sequence))
    }
}

/// Assigns epochs and ingress sequences for one source, and measures
/// run-relative monotonic time.
///
/// The source owner calls [`Self::begin_epoch`] once per connection attempt,
/// including attempts that then fail, and [`Self::next_key`] once per raw or
/// lifecycle event in the order it emits them. Epochs start at 1 and are
/// never reused; each epoch's ingress sequence starts at 0.
#[derive(Debug)]
pub struct SourceSequencer {
    identity: SourceIdentity,
    epoch: u64,
    next_sequence: u64,
    origin: Instant,
}

impl SourceSequencer {
    /// Start a sequencer for `identity`. The monotonic clock origin is now.
    /// Keys issued before the first [`Self::begin_epoch`] belong to epoch 0.
    pub fn new(identity: SourceIdentity) -> Self {
        Self {
            identity,
            epoch: 0,
            next_sequence: 0,
            origin: Instant::now(),
        }
    }

    /// The source identity.
    pub fn identity(&self) -> &SourceIdentity {
        &self.identity
    }

    /// The current epoch.
    pub fn epoch(&self) -> ConnectionEpoch {
        ConnectionEpoch(self.epoch)
    }

    /// Begin a new connection attempt: a fresh epoch whose sequence restarts
    /// at 0.
    pub fn begin_epoch(&mut self) -> ConnectionEpoch {
        self.epoch += 1;
        self.next_sequence = 0;
        ConnectionEpoch(self.epoch)
    }

    /// The key for the next event in the current epoch.
    pub fn next_key(&mut self) -> SourceKey {
        let key = SourceKey::new(self.identity.clone(), self.epoch(), self.next_sequence);
        self.next_sequence += 1;
        key
    }

    /// Nanoseconds since this sequencer's monotonic origin.
    pub fn elapsed_nanos_now(&self) -> MonotonicElapsed {
        MonotonicElapsed::from_nanos(
            u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX),
        )
    }
}

/// Application-message kind of a raw observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PayloadKind {
    /// A binary WebSocket message.
    Binary,
    /// A text WebSocket message.
    Text,
}

/// Receive wall time: signed UTC Unix nanoseconds.
///
/// The value is read from the system wall clock when the message is handed to
/// the source owner. Its representation has nanosecond resolution; the
/// clock's actual accuracy and resolution are platform-dependent and not
/// asserted. The wall clock can move backwards; compare
/// [`MonotonicElapsed`] for local ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ReceiveTime {
    unix_nanos: i64,
}

impl ReceiveTime {
    /// Wrap an explicit timestamp.
    pub fn from_unix_nanos(unix_nanos: i64) -> Self {
        Self { unix_nanos }
    }

    /// The current system time. Times before 1970 are negative.
    pub fn now() -> Self {
        let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
            Err(e) => -i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX),
        };
        Self { unix_nanos: nanos }
    }

    /// Signed UTC Unix nanoseconds.
    pub fn unix_nanos(self) -> i64 {
        self.unix_nanos
    }
}

/// Monotonic time since the source's run-local origin, in nanoseconds.
///
/// Not comparable across runs, and never a serialized `Instant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MonotonicElapsed {
    nanos: u64,
}

impl MonotonicElapsed {
    /// Wrap an explicit duration from the run origin.
    pub fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
    }

    /// Nanoseconds from the run origin.
    pub fn nanos(self) -> u64 {
        self.nanos
    }
}

/// Immutable, shared payload bytes.
///
/// A payload may be a sub-range of a larger shared allocation; cloning shares
/// that allocation. [`Self::retained_bytes`] reports the size of the whole
/// allocation kept alive, which is what bounded queues charge, while
/// [`Self::len`] is the visible length.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Payload {
    // The received buffer itself, shared: taking ownership never copies it.
    buf: Arc<Vec<u8>>,
    start: usize,
    end: usize,
}

impl Payload {
    /// Take ownership of `bytes` without copying them.
    pub fn new(bytes: Vec<u8>) -> Self {
        let end = bytes.len();
        Self {
            buf: Arc::new(bytes),
            start: 0,
            end,
        }
    }

    /// The visible bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[self.start..self.end]
    }

    /// Visible length.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether the visible range is empty.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Bytes of the backing allocation kept alive by this payload: its
    /// full capacity, which can exceed the received length.
    pub fn retained_bytes(&self) -> usize {
        self.buf.capacity()
    }

    /// A sub-range sharing the same allocation, or `None` if out of bounds.
    pub fn slice(&self, range: std::ops::Range<usize>) -> Option<Self> {
        if range.start > range.end || range.end > self.len() {
            return None;
        }
        Some(Self {
            buf: self.buf.clone(),
            start: self.start + range.start,
            end: self.start + range.end,
        })
    }
}

impl fmt::Debug for Payload {
    // Payload bytes are never dumped by default diagnostics.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Payload({} bytes)", self.len())
    }
}

impl Serialize for Payload {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(self.as_bytes())
    }
}

impl<'de> Deserialize<'de> for Payload {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::new(Vec::<u8>::deserialize(d)?))
    }
}

/// One application message exactly as delivered: bytes and kind unchanged.
///
/// Fields are private and there are no setters, so a decoder or diagnostic
/// holding a reference cannot overwrite the source identity or payload:
///
/// ```compile_fail
/// # use manja::kite::envelope::*;
/// # fn f(obs: &mut RawObservation, other: SourceKey) {
/// obs.source = other;
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawObservation {
    version: EnvelopeVersion,
    source: SourceKey,
    kind: PayloadKind,
    received_at: ReceiveTime,
    elapsed: MonotonicElapsed,
    payload: Payload,
}

impl RawObservation {
    /// Wrap one received application message.
    ///
    /// Fails if `payload` exceeds `max_payload_bytes`, or if a text payload is
    /// not UTF-8. `max_payload_bytes` is itself capped at
    /// [`MAX_PAYLOAD_BYTES_LIMIT`].
    pub fn new(
        source: SourceKey,
        kind: PayloadKind,
        received_at: ReceiveTime,
        elapsed: MonotonicElapsed,
        payload: impl Into<Payload>,
        max_payload_bytes: usize,
    ) -> Result<Self, EnvelopeError> {
        let payload = payload.into();
        let max = max_payload_bytes.min(MAX_PAYLOAD_BYTES_LIMIT);
        if payload.len() > max {
            return Err(EnvelopeError::PayloadTooLarge {
                len: payload.len(),
                max,
            });
        }
        if kind == PayloadKind::Text && std::str::from_utf8(payload.as_bytes()).is_err() {
            return Err(EnvelopeError::InvalidText);
        }
        Ok(Self {
            version: ENVELOPE_VERSION,
            source,
            kind,
            received_at,
            elapsed,
            payload,
        })
    }

    /// Envelope version.
    pub fn version(&self) -> EnvelopeVersion {
        self.version
    }

    /// Source key.
    pub fn source(&self) -> &SourceKey {
        &self.source
    }

    /// Binary or text.
    pub fn kind(&self) -> PayloadKind {
        self.kind
    }

    /// Receive wall time.
    pub fn received_at(&self) -> ReceiveTime {
        self.received_at
    }

    /// Monotonic time since the run origin.
    pub fn elapsed(&self) -> MonotonicElapsed {
        self.elapsed
    }

    /// The payload, unchanged.
    pub fn payload(&self) -> &Payload {
        &self.payload
    }

    /// The payload as text, for text observations.
    pub fn text(&self) -> Option<&str> {
        match self.kind {
            // Checked valid UTF-8 at construction.
            PayloadKind::Text => std::str::from_utf8(self.payload.as_bytes()).ok(),
            PayloadKind::Binary => None,
        }
    }
}

impl From<Vec<u8>> for Payload {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<String> for Payload {
    fn from(text: String) -> Self {
        Self::new(text.into_bytes())
    }
}

#[derive(Deserialize)]
struct RawObservationRepr {
    version: EnvelopeVersion,
    source: SourceKey,
    kind: PayloadKind,
    received_at: ReceiveTime,
    elapsed: MonotonicElapsed,
    payload: Payload,
}

impl<'de> Deserialize<'de> for RawObservation {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = RawObservationRepr::deserialize(d)?;
        r.version
            .check_supported()
            .map_err(serde::de::Error::custom)?;
        if r.kind == PayloadKind::Text && std::str::from_utf8(r.payload.as_bytes()).is_err() {
            return Err(serde::de::Error::custom(EnvelopeError::InvalidText));
        }
        Ok(Self {
            version: r.version,
            source: r.source,
            kind: r.kind,
            received_at: r.received_at,
            elapsed: r.elapsed,
            payload: r.payload,
        })
    }
}

/// Why a connection ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DisconnectReason {
    /// The stream ended without a close frame.
    Eof,
    /// The peer sent a close frame.
    RemoteClose,
    /// No message or heartbeat arrived within the liveness timeout.
    LivenessTimeout,
    /// A transport or send failure.
    TransportError,
    /// The peer violated the WebSocket protocol.
    ProtocolError,
    /// The source owner stopped delivery because the primary consumer did
    /// not accept observations within its bounds.
    DeliveryOverload,
    /// Requested shutdown.
    Shutdown,
}

/// Known bounds of a possible gap in observations, without invented counts.
///
/// A reconnect does not backfill anything. The protocol supplies no server
/// sequence, so the number of missed messages is unknown; these fields state
/// only what the source owner saw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapFacts {
    /// Epoch whose connection ended.
    pub previous_epoch: ConnectionEpoch,
    /// Last ingress sequence issued in that epoch, if any event was issued.
    pub last_sequence_in_previous_epoch: Option<u64>,
    /// Monotonic time at which the previous connection was seen to end.
    pub disconnected_at: MonotonicElapsed,
    /// Monotonic time at which the new connection was established.
    pub reconnected_at: MonotonicElapsed,
    /// Why the previous connection ended.
    pub reason: DisconnectReason,
}

/// A fact about the source itself, in source order with raw observations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LifecycleKind {
    /// A connection attempt began under a fresh epoch.
    ConnectAttempt {
        /// Attempt number within the current outage, starting at 1.
        attempt: u32,
    },
    /// The WebSocket handshake succeeded. This is not subscription
    /// readiness.
    Connected,
    /// Subscription commands for a desired-state revision were written to
    /// the socket. This is not broker acknowledgement.
    CommandsSent {
        /// Desired-state revision.
        revision: u64,
    },
    /// A revision was replaced by a later one before its commands were sent.
    Superseded {
        /// The superseded revision.
        revision: u64,
    },
    /// Writing a revision's commands failed.
    SendFailed {
        /// The revision that failed.
        revision: u64,
    },
    /// Restoration finished for a revision; the connection is active.
    /// Freshness of any instrument is not implied.
    Active {
        /// Desired-state revision in effect.
        revision: u64,
    },
    /// The connection ended.
    Disconnected {
        /// Why.
        reason: DisconnectReason,
    },
    /// A known possible gap between two connections.
    Gap(GapFacts),
    /// Waiting before the next attempt.
    Backoff {
        /// Delay before the next attempt, in milliseconds.
        delay_ms: u64,
        /// Attempt number that will follow.
        next_attempt: u32,
    },
    /// The broker rejected the supplied credentials; terminal.
    AuthRejected {
        /// HTTP status of the rejected handshake.
        http_status: u16,
    },
    /// The source failed permanently; terminal.
    Failed,
    /// The source stopped after a requested shutdown.
    Stopped,
}

/// A source-lifecycle event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LifecycleEvent {
    version: EnvelopeVersion,
    source: SourceKey,
    received_at: ReceiveTime,
    elapsed: MonotonicElapsed,
    kind: LifecycleKind,
}

impl LifecycleEvent {
    /// Build a lifecycle event.
    pub fn new(
        source: SourceKey,
        received_at: ReceiveTime,
        elapsed: MonotonicElapsed,
        kind: LifecycleKind,
    ) -> Self {
        Self {
            version: ENVELOPE_VERSION,
            source,
            received_at,
            elapsed,
            kind,
        }
    }

    /// Envelope version.
    pub fn version(&self) -> EnvelopeVersion {
        self.version
    }

    /// Source key, in the same ordering as raw observations.
    pub fn source(&self) -> &SourceKey {
        &self.source
    }

    /// Wall time at which the owner recorded the event.
    pub fn received_at(&self) -> ReceiveTime {
        self.received_at
    }

    /// Monotonic time since the run origin.
    pub fn elapsed(&self) -> MonotonicElapsed {
        self.elapsed
    }

    /// What happened.
    pub fn kind(&self) -> &LifecycleKind {
        &self.kind
    }
}

#[derive(Deserialize)]
struct LifecycleEventRepr {
    version: EnvelopeVersion,
    source: SourceKey,
    received_at: ReceiveTime,
    elapsed: MonotonicElapsed,
    kind: LifecycleKind,
}

impl<'de> Deserialize<'de> for LifecycleEvent {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = LifecycleEventRepr::deserialize(d)?;
        r.version
            .check_supported()
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            version: r.version,
            source: r.source,
            received_at: r.received_at,
            elapsed: r.elapsed,
            kind: r.kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(seq: &mut SourceSequencer, bytes: &[u8]) -> RawObservation {
        RawObservation::new(
            seq.next_key(),
            PayloadKind::Binary,
            ReceiveTime::from_unix_nanos(1),
            seq.elapsed_nanos_now(),
            bytes.to_vec(),
            DEFAULT_MAX_PAYLOAD_BYTES,
        )
        .unwrap()
    }

    #[test]
    fn identical_payloads_have_distinct_source_keys() {
        let mut seq = SourceSequencer::new(SourceIdentity::generate());
        seq.begin_epoch();
        let a = obs(&mut seq, &[1, 2, 3]);
        let b = obs(&mut seq, &[1, 2, 3]);
        assert_eq!(a.payload(), b.payload());
        assert_ne!(a.source(), b.source());
        assert_eq!(b.source().ingress_sequence(), 1);
    }

    #[test]
    fn every_attempt_gets_a_fresh_epoch_and_shares_ordering() {
        let mut seq = SourceSequencer::new(SourceIdentity::generate());
        // Attempt 1 fails during the handshake: its epoch is still consumed.
        let e1 = seq.begin_epoch();
        let attempt = LifecycleEvent::new(
            seq.next_key(),
            ReceiveTime::now(),
            seq.elapsed_nanos_now(),
            LifecycleKind::ConnectAttempt { attempt: 1 },
        );
        let failed = LifecycleEvent::new(
            seq.next_key(),
            ReceiveTime::now(),
            seq.elapsed_nanos_now(),
            LifecycleKind::Disconnected {
                reason: DisconnectReason::TransportError,
            },
        );
        // Attempt 2 succeeds; its lifecycle and raw events share one sequence.
        let e2 = seq.begin_epoch();
        assert!(e2 > e1);
        let connected = LifecycleEvent::new(
            seq.next_key(),
            ReceiveTime::now(),
            seq.elapsed_nanos_now(),
            LifecycleKind::Connected,
        );
        let raw = obs(&mut seq, &[0x01]);
        assert_eq!(attempt.source().connection_epoch(), e1);
        assert_eq!(failed.source().ingress_sequence(), 1);
        assert_eq!(connected.source().connection_epoch(), e2);
        assert_eq!(connected.source().ingress_sequence(), 0);
        assert_eq!(raw.source().ingress_sequence(), 1);
    }

    #[test]
    fn exact_bytes_are_preserved_including_heartbeat_and_unknown_text() {
        let mut seq = SourceSequencer::new(SourceIdentity::generate());
        seq.begin_epoch();
        let heartbeat = obs(&mut seq, &[0x01]);
        assert_eq!(heartbeat.payload().as_bytes(), &[0x01]);
        let text = r#"{"type":"never-seen","data":"é"}"#;
        let t = RawObservation::new(
            seq.next_key(),
            PayloadKind::Text,
            ReceiveTime::now(),
            seq.elapsed_nanos_now(),
            text.to_string(),
            DEFAULT_MAX_PAYLOAD_BYTES,
        )
        .unwrap();
        assert_eq!(t.text(), Some(text));
        assert_eq!(t.payload().as_bytes(), text.as_bytes());
    }

    #[test]
    fn payload_bound_and_text_validity_are_enforced() {
        let mut seq = SourceSequencer::new(SourceIdentity::generate());
        let err = RawObservation::new(
            seq.next_key(),
            PayloadKind::Binary,
            ReceiveTime::now(),
            seq.elapsed_nanos_now(),
            vec![0; 17],
            16,
        )
        .unwrap_err();
        assert_eq!(err, EnvelopeError::PayloadTooLarge { len: 17, max: 16 });
        let err = RawObservation::new(
            seq.next_key(),
            PayloadKind::Text,
            ReceiveTime::now(),
            seq.elapsed_nanos_now(),
            vec![0xff],
            16,
        )
        .unwrap_err();
        assert_eq!(err, EnvelopeError::InvalidText);
    }

    #[test]
    fn round_trips_through_serde_and_rejects_unknown_major() {
        let mut seq = SourceSequencer::new(SourceIdentity::generate());
        seq.begin_epoch();
        let a = obs(&mut seq, &[0, 1, 0, 8]);
        let json = serde_json::to_string(&a).unwrap();
        let back: RawObservation = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
        // No Instant, socket or task handle leaks into the portable form.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "elapsed",
                "kind",
                "payload",
                "received_at",
                "source",
                "version"
            ]
        );
        let bumped = json.replace(r#""major":1"#, r#""major":2"#);
        let err = serde_json::from_str::<RawObservation>(&bumped).unwrap_err();
        assert!(err.to_string().contains("unsupported envelope version 2.0"));
        // A newer minor of the same major is readable.
        let minor = json.replace(r#""minor":0"#, r#""minor":7"#);
        assert!(serde_json::from_str::<RawObservation>(&minor).is_ok());
    }

    #[test]
    fn wall_clock_regression_is_representable() {
        // The UTC field moves backwards while monotonic time does not.
        let mut seq = SourceSequencer::new(SourceIdentity::generate());
        seq.begin_epoch();
        let a = RawObservation::new(
            seq.next_key(),
            PayloadKind::Binary,
            ReceiveTime::from_unix_nanos(2_000),
            MonotonicElapsed::from_nanos(10),
            vec![1],
            16,
        )
        .unwrap();
        let b = RawObservation::new(
            seq.next_key(),
            PayloadKind::Binary,
            ReceiveTime::from_unix_nanos(1_000),
            MonotonicElapsed::from_nanos(11),
            vec![1],
            16,
        )
        .unwrap();
        assert!(b.received_at() < a.received_at());
        assert!(b.elapsed() > a.elapsed());
    }

    #[test]
    fn identifiers_are_bounded() {
        assert!(SourceIdentity::new("", RunId::from_u128(1), "f").is_err());
        assert!(SourceIdentity::new("p", RunId::from_u128(1), "x".repeat(65)).is_err());
        assert!(SourceIdentity::new("p q", RunId::from_u128(1), "f").is_err());
        assert_ne!(RunId::generate(), RunId::generate());
    }

    #[test]
    fn slices_charge_the_retained_allocation() {
        let big = Payload::new(vec![7; 1 << 20]);
        let small = big.slice(0..8).unwrap();
        assert_eq!(small.len(), 8);
        assert_eq!(small.retained_bytes(), 1 << 20);
        let (start, end) = (4, 2);
        assert!(big.slice(start..end).is_none());
        assert!(big.slice(0..(1 << 20) + 1).is_none());
    }

    #[test]
    fn gap_facts_state_bounds_without_counts() {
        let gap = GapFacts {
            previous_epoch: ConnectionEpoch(1),
            last_sequence_in_previous_epoch: Some(41),
            disconnected_at: MonotonicElapsed::from_nanos(5),
            reconnected_at: MonotonicElapsed::from_nanos(9),
            reason: DisconnectReason::Eof,
        };
        let json = serde_json::to_value(LifecycleKind::Gap(gap)).unwrap();
        assert!(json.get("missing").is_none());
        assert_eq!(json["kind"], "gap");
    }
}
