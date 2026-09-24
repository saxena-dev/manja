//! The provenance record wrapped around every ticker message.
//!
//! Each message the ticker receives arrives as a [`RawObservation`]: the
//! exact bytes, when they were received, and a [`SourceKey`] saying which
//! feed, connection and position in the stream they came from. Changes in
//! the connection arrive as [`LifecycleEvent`]s in the same stream. Together
//! they let you tell a fresh connection from a gap, and replay captured data
//! with the same provenance it had live.
//!
//! These types are serializable and need no async runtime, so you can store
//! observations and decode them later.
//!
pub mod types;

pub use types::{
    ConnectionEpoch, DEFAULT_MAX_PAYLOAD_BYTES, DisconnectReason, ENVELOPE_VERSION, EnvelopeError,
    EnvelopeVersion, GapFacts, LifecycleEvent, LifecycleKind, MAX_ID_BYTES,
    MAX_PAYLOAD_BYTES_LIMIT, MonotonicElapsed, Payload, PayloadKind, RawObservation, ReceiveTime,
    RunId, SourceIdentity, SourceKey, SourceSequencer,
};
