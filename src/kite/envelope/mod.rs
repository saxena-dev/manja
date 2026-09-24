//! Portable raw-observation and source-lifecycle envelopes.
//!
//! Compiled in every feature build, with no async runtime. `types` holds the
//! versioned envelope and its source identity; its items are re-exported here.
//!
pub mod types;

pub use types::{
    ConnectionEpoch, DisconnectReason, EnvelopeError, EnvelopeVersion, GapFacts, LifecycleEvent,
    LifecycleKind, MonotonicElapsed, Payload, PayloadKind, RawObservation, ReceiveTime, RunId,
    SourceIdentity, SourceKey, SourceSequencer, DEFAULT_MAX_PAYLOAD_BYTES, ENVELOPE_VERSION,
    MAX_ID_BYTES, MAX_PAYLOAD_BYTES_LIMIT,
};
