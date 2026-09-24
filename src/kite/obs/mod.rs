//! Intrinsic, exporter-independent library instrumentation.
//!
//! Compiled in every feature build, including decoder-only, with no async
//! runtime. The host application owns subscribers, recorders, exporters,
//! dashboards and alerting; this module owns what the SDK emits:
//!
//! - [`handle`]: the [`Observability`] handle that defines a recording
//!   scope, the [`MetricRecorder`] trait, scope-aggregated gauges, an
//!   [`InMemoryRecorder`] and a bounded [`BridgeRecorder`].
//! - [`schema`]: the frozen span and metric catalogues, closed label domains
//!   and histogram buckets, version [`OBS_SCHEMA_VERSION`].
//! - [`diagnostics`]: bounded, redacted text and failure history used by the
//!   typed status and diagnostic snapshots.
//!
//! Instrumentation never blocks protocol progress: with no subscriber and no
//! recorder, operations behave identically and typed diagnostics remain
//! available.
//!
pub mod diagnostics;
pub mod handle;
pub mod schema;

pub use handle::{
    AgeGuard, BridgeRecord, BridgeRecorder, GaugeGuard, GaugeSample, InMemoryRecorder, Labels,
    MAX_STATIC_DIMENSIONS, MetricRecorder, ObsConfigError, Observability, ObservabilityBuilder,
};
pub use schema::{Instrument, InstrumentKind, OBS_SCHEMA_VERSION};
