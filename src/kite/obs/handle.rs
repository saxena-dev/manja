//! The construction-time observability handle and recording interfaces.
//!
//! [`Observability`] defines one recording scope: a host-supplied
//! [`MetricRecorder`] (or none), up to [`MAX_STATIC_DIMENSIONS`] fixed static
//! dimensions, and the scope's gauge state. Clones share the scope. Nothing
//! here installs a global subscriber or recorder, starts a task, opens a
//! socket or listener, or waits for an exporter; it works with no async
//! runtime at all.
//!
//! [`Observability::disabled`] is the default everywhere: no recorder, so
//! every record call returns after one branch, with no formatting and no
//! allocation. Tracing spans are emitted through `tracing` and cost nothing
//! beyond `tracing`'s own disabled check when the host installed no
//! subscriber.
//!
//! Gauges are aggregated inside the scope: each contributor holds a
//! [`GaugeGuard`] whose value is summed per series, and dropping the guard
//! removes its contribution, so a closed client or ticker leaves no ghost
//! gauge. Oldest-queue-age gauges are computed at collection time, as the
//! maximum over live contributors, so the age keeps advancing when progress
//! stops ([`Observability::collect_gauges`]).
//!
//! ```
//! use std::sync::Arc;
//! use manja::kite::obs::{InMemoryRecorder, Observability};
//!
//! // No collector: everything still works.
//! let quiet = Observability::disabled();
//! assert!(!quiet.is_recording());
//!
//! // A host-owned recorder, shared by every clone of the handle.
//! let recorder = Arc::new(InMemoryRecorder::new());
//! let obs = Observability::with_recorder(recorder.clone());
//! assert!(obs.clone().is_recording());
//! ```
//!
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use crate::kite::obs::schema::*;

/// Maximum number of static dimensions per scope.
pub const MAX_STATIC_DIMENSIONS: usize = 4;

/// A fixed-size set of label values for one instrument, in the order of
/// [`Instrument::label_keys`]. Values are always from closed domains.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Labels {
    instrument: Instrument,
    values: [&'static str; MAX_LABELS],
    len: u8,
}

impl Labels {
    fn new(instrument: Instrument, values: &[&'static str]) -> Self {
        debug_assert_eq!(values.len(), instrument.label_keys().len());
        let mut out = [""; MAX_LABELS];
        out[..values.len()].copy_from_slice(values);
        Self {
            instrument,
            values: out,
            len: values.len() as u8,
        }
    }

    /// The instrument these labels belong to.
    pub fn instrument(&self) -> Instrument {
        self.instrument
    }

    /// `(key, value)` pairs, in catalogue order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
        self.instrument
            .label_keys()
            .iter()
            .zip(&self.values[..self.len as usize])
            .map(|(k, v)| (k.as_str(), *v))
    }

    /// Label values only, in catalogue order.
    pub fn values(&self) -> &[&'static str] {
        &self.values[..self.len as usize]
    }

    // --- typed constructors: one per label subset of the catalogue ---

    pub(crate) fn http_operation(
        instrument: Instrument,
        m: Method,
        e: Endpoint,
        q: QuotaClass,
        r: HttpOperationResult,
    ) -> Self {
        Self::new(
            instrument,
            &[m.as_str(), e.as_str(), q.as_str(), r.as_str()],
        )
    }

    pub(crate) fn http_attempt(
        instrument: Instrument,
        m: Method,
        e: Endpoint,
        r: HttpAttemptResult,
    ) -> Self {
        Self::new(instrument, &[m.as_str(), e.as_str(), r.as_str()])
    }

    pub(crate) fn http_retry(m: Method, e: Endpoint, c: RetryCause) -> Self {
        Self::new(
            Instrument::HttpRetriesTotal,
            &[m.as_str(), e.as_str(), c.as_str()],
        )
    }

    pub(crate) fn quota(instrument: Instrument, q: QuotaClass) -> Self {
        Self::new(instrument, &[q.as_str()])
    }

    pub(crate) fn admission_wait(q: QuotaClass, r: AdmissionResult) -> Self {
        Self::new(Instrument::HttpAdmissionWait, &[q.as_str(), r.as_str()])
    }

    pub(crate) fn auth_rejection(t: TransportLabel) -> Self {
        Self::new(Instrument::AuthRejectionsTotal, &[t.as_str()])
    }

    pub(crate) fn connection(instrument: Instrument, r: ConnectionResult) -> Self {
        Self::new(instrument, &[r.as_str()])
    }

    pub(crate) fn none(instrument: Instrument) -> Self {
        Self::new(instrument, &[])
    }

    pub(crate) fn reconnect(r: ReconnectReason) -> Self {
        Self::new(Instrument::TickerReconnectsTotal, &[r.as_str()])
    }

    pub(crate) fn command(c: CommandKind, d: Decision) -> Self {
        Self::new(Instrument::TickerCommandsTotal, &[c.as_str(), d.as_str()])
    }

    pub(crate) fn restore(r: RestoreResult) -> Self {
        Self::new(Instrument::TickerRestoreDuration, &[r.as_str()])
    }

    pub(crate) fn payload(instrument: Instrument, p: PayloadKindLabel) -> Self {
        Self::new(instrument, &[p.as_str()])
    }

    pub(crate) fn queue(instrument: Instrument, q: QueueRole) -> Self {
        Self::new(instrument, &[q.as_str()])
    }

    pub(crate) fn decode_batch(s: SourceMode, p: PayloadKindLabel, r: DecodeResult) -> Self {
        Self::new(
            Instrument::DecodeBatchesTotal,
            &[s.as_str(), p.as_str(), r.as_str()],
        )
    }

    pub(crate) fn decode_duration(s: SourceMode, p: PayloadKindLabel) -> Self {
        Self::new(Instrument::DecodeDuration, &[s.as_str(), p.as_str()])
    }

    pub(crate) fn shutdown(r: ShutdownResult) -> Self {
        Self::new(Instrument::TickerShutdownDuration, &[r.as_str()])
    }
}

impl fmt::Debug for Labels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// A host-supplied metric sink.
///
/// Calls arrive on SDK hot paths (HTTP dispatch, socket receive, command
/// handling, decoding), so implementations must be bounded and non-blocking
/// and must not perform I/O; exporter work belongs behind a bounded adapter
/// such as [`BridgeRecorder`]. The SDK does not isolate an implementation that
/// blocks or panics: such an implementation violates this contract.
pub trait MetricRecorder: Send + Sync + 'static {
    /// Add `value` to a counter.
    fn counter_add(&self, labels: &Labels, value: u64);
    /// Set a gauge to the scope's aggregated value.
    fn gauge_set(&self, labels: &Labels, value: f64);
    /// Record one histogram observation, in seconds.
    fn histogram_record(&self, labels: &Labels, seconds: f64);
}

/// Why an observability configuration was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObsConfigError {
    /// More than [`MAX_STATIC_DIMENSIONS`] static dimensions.
    TooManyDimensions,
    /// A key or value was empty, longer than 64 bytes, or not printable
    /// ASCII.
    InvalidDimension,
    /// A bounded buffer capacity was outside its permitted range.
    Capacity {
        /// Smallest permitted value.
        min: usize,
        /// Largest permitted value.
        max: usize,
    },
}

impl fmt::Display for ObsConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyDimensions => write!(f, "at most {MAX_STATIC_DIMENSIONS} dimensions"),
            Self::InvalidDimension => f.write_str("invalid static dimension"),
            Self::Capacity { min, max } => write!(f, "capacity must be in {min}..={max}"),
        }
    }
}

impl std::error::Error for ObsConfigError {}

// Gauge series whose contributions are summed within a scope.
const SUM_GAUGES: &[Instrument] = &[
    Instrument::HttpInFlight,
    Instrument::HttpAdmissionWaiters,
    Instrument::TickerConnectionsActive,
    Instrument::SdkQueueMessages,
    Instrument::SdkQueueRetainedBytes,
];

#[derive(Default)]
struct GaugeState {
    sums: BTreeMap<Labels, i64>,
    ages: BTreeMap<Labels, Vec<Weak<AgeCell>>>,
}

struct Inner {
    recorder: Option<Arc<dyn MetricRecorder>>,
    dimensions: Vec<(String, String)>,
    gauges: Mutex<GaugeState>,
    next_operation_id: AtomicU64,
}

/// A cloneable observability handle: one recording scope.
#[derive(Clone)]
pub struct Observability(Arc<Inner>);

impl fmt::Debug for Observability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Observability")
            .field("recording", &self.is_recording())
            .field("dimensions", &self.0.dimensions)
            .finish()
    }
}

impl Default for Observability {
    fn default() -> Self {
        Self::disabled()
    }
}

/// Builder for [`Observability`].
#[derive(Default)]
pub struct ObservabilityBuilder {
    recorder: Option<Arc<dyn MetricRecorder>>,
    dimensions: Vec<(String, String)>,
}

impl ObservabilityBuilder {
    /// Send metrics to `recorder`.
    pub fn recorder(mut self, recorder: Arc<dyn MetricRecorder>) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Add a fixed static dimension, such as `("client", "primary")`, that
    /// host adapters may attach to every series of this scope.
    pub fn static_dimension(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, ObsConfigError> {
        let (key, value) = (key.into(), value.into());
        let ok = |s: &str| {
            !s.is_empty()
                && s.len() <= MAX_LABEL_VALUE_BYTES
                && s.bytes().all(|b| b.is_ascii_graphic())
        };
        if !ok(&key) || !ok(&value) {
            return Err(ObsConfigError::InvalidDimension);
        }
        if self.dimensions.len() == MAX_STATIC_DIMENSIONS {
            return Err(ObsConfigError::TooManyDimensions);
        }
        self.dimensions.push((key, value));
        Ok(self)
    }

    /// Build the handle.
    pub fn build(self) -> Observability {
        Observability(Arc::new(Inner {
            recorder: self.recorder,
            dimensions: self.dimensions,
            gauges: Mutex::new(GaugeState::default()),
            next_operation_id: AtomicU64::new(1),
        }))
    }
}

/// One gauge value at collection time.
#[derive(Clone, Debug, PartialEq)]
pub struct GaugeSample {
    /// Instrument and label values.
    pub labels: Labels,
    /// Aggregated value: a sum, or for oldest age the maximum in seconds.
    pub value: f64,
}

impl Observability {
    /// No recorder: the default for every constructor.
    pub fn disabled() -> Self {
        ObservabilityBuilder::default().build()
    }

    /// A handle recording into `recorder`.
    pub fn with_recorder(recorder: Arc<dyn MetricRecorder>) -> Self {
        Self::builder().recorder(recorder).build()
    }

    /// A builder.
    pub fn builder() -> ObservabilityBuilder {
        ObservabilityBuilder::default()
    }

    /// Whether a recorder is attached.
    pub fn is_recording(&self) -> bool {
        self.0.recorder.is_some()
    }

    /// The scope's static dimensions.
    pub fn static_dimensions(&self) -> &[(String, String)] {
        &self.0.dimensions
    }

    /// Whether two handles share one recording scope.
    pub fn same_scope(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// A scope-unique, never-reused operation identifier for trace fields.
    /// Never a metric label.
    pub(crate) fn next_operation_id(&self) -> u64 {
        self.0.next_operation_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn counter(&self, labels: Labels, value: u64) {
        if let Some(r) = &self.0.recorder {
            r.counter_add(&labels, value);
        }
    }

    pub(crate) fn histogram(&self, labels: Labels, seconds: f64) {
        if let Some(r) = &self.0.recorder {
            r.histogram_record(&labels, seconds);
        }
    }

    /// Contribute to a summed gauge; the contribution is removed on drop.
    pub(crate) fn gauge(&self, labels: Labels) -> GaugeGuard {
        debug_assert!(SUM_GAUGES.contains(&labels.instrument()));
        GaugeGuard {
            obs: self.clone(),
            labels,
            value: AtomicI64::new(0),
        }
    }

    /// Contribute to an oldest-age gauge; the contribution is removed on drop.
    pub(crate) fn age(&self, role: QueueRole) -> AgeGuard {
        let labels = Labels::queue(Instrument::SdkQueueOldestAge, role);
        let cell = Arc::new(AgeCell(Mutex::new(None)));
        let mut state = self.0.gauges.lock().unwrap_or_else(|e| e.into_inner());
        let list = state.ages.entry(labels).or_default();
        list.retain(|w| w.strong_count() > 0);
        list.push(Arc::downgrade(&cell));
        AgeGuard(cell)
    }

    fn adjust(&self, labels: Labels, delta: i64) {
        if delta == 0 {
            return;
        }
        let total = {
            let mut state = self.0.gauges.lock().unwrap_or_else(|e| e.into_inner());
            let entry = state.sums.entry(labels).or_insert(0);
            *entry += delta;
            *entry
        };
        if let Some(r) = &self.0.recorder {
            r.gauge_set(&labels, total as f64);
        }
    }

    /// Current aggregated gauges of this scope, with oldest ages computed
    /// now. Works without a recorder.
    pub fn collect_gauges(&self) -> Vec<GaugeSample> {
        let now = Instant::now();
        let mut state = self.0.gauges.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<GaugeSample> = state
            .sums
            .iter()
            .map(|(labels, v)| GaugeSample {
                labels: *labels,
                value: *v as f64,
            })
            .collect();
        for (labels, cells) in state.ages.iter_mut() {
            cells.retain(|w| w.strong_count() > 0);
            let oldest = cells
                .iter()
                .filter_map(|w| w.upgrade())
                .filter_map(|c| *c.0.lock().unwrap_or_else(|e| e.into_inner()))
                .map(|t| now.saturating_duration_since(t).as_secs_f64())
                .fold(0.0, f64::max);
            out.push(GaugeSample {
                labels: *labels,
                value: oldest,
            });
        }
        if let Some(r) = &self.0.recorder {
            for s in out
                .iter()
                .filter(|s| s.labels.instrument() == Instrument::SdkQueueOldestAge)
            {
                r.gauge_set(&s.labels, s.value);
            }
        }
        out
    }

    /// The aggregated value of one gauge series, or 0.
    pub fn gauge_value(&self, labels: &Labels) -> f64 {
        self.collect_gauges()
            .into_iter()
            .find(|s| &s.labels == labels)
            .map_or(0.0, |s| s.value)
    }
}

/// One contribution to a summed gauge. Dropping it removes the contribution.
pub struct GaugeGuard {
    obs: Observability,
    labels: Labels,
    value: AtomicI64,
}

impl GaugeGuard {
    /// Set this contributor's value.
    pub(crate) fn set(&self, value: i64) {
        let old = self.value.swap(value, Ordering::Relaxed);
        self.obs.adjust(self.labels, value - old);
    }

    /// Add `delta` to this contributor's value.
    pub(crate) fn add(&self, delta: i64) {
        self.value.fetch_add(delta, Ordering::Relaxed);
        self.obs.adjust(self.labels, delta);
    }
}

impl Drop for GaugeGuard {
    fn drop(&mut self) {
        let old = self.value.swap(0, Ordering::Relaxed);
        self.obs.adjust(self.labels, -old);
    }
}

impl fmt::Debug for GaugeGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GaugeGuard({:?})", self.labels)
    }
}

struct AgeCell(Mutex<Option<Instant>>);

/// One contribution to an oldest-age gauge. Dropping it removes the
/// contribution.
pub struct AgeGuard(Arc<AgeCell>);

impl AgeGuard {
    /// Record the enqueue time of this contributor's oldest item, or `None`
    /// when it holds nothing.
    pub(crate) fn set_oldest(&self, oldest: Option<Instant>) {
        *self.0 .0.lock().unwrap_or_else(|e| e.into_inner()) = oldest;
    }
}

impl fmt::Debug for AgeGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AgeGuard")
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Series {
    Counter(u64),
    Gauge(f64),
    Histogram {
        buckets: [u64; HISTOGRAM_BUCKETS_SECONDS.len() + 1],
        count: u64,
        sum: f64,
    },
}

/// An in-memory recorder for tests and simple hosts.
///
/// It keeps one bounded entry per series (histograms as bucket counts), so its
/// memory is bounded by the catalogue's series bounds.
#[derive(Default)]
pub struct InMemoryRecorder {
    series: Mutex<BTreeMap<Labels, Series>>,
}

impl InMemoryRecorder {
    /// An empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    fn find(&self, instrument: Instrument, values: &[&str]) -> Option<Series> {
        let series = self.series.lock().unwrap_or_else(|e| e.into_inner());
        series
            .iter()
            .find(|(l, _)| l.instrument() == instrument && l.values() == values)
            .map(|(_, s)| s.clone())
    }

    /// A counter's value, or 0.
    pub fn counter(&self, instrument: Instrument, values: &[&str]) -> u64 {
        match self.find(instrument, values) {
            Some(Series::Counter(v)) => v,
            _ => 0,
        }
    }

    /// Sum of a counter over every label combination.
    pub fn counter_total(&self, instrument: Instrument) -> u64 {
        let series = self.series.lock().unwrap_or_else(|e| e.into_inner());
        series
            .iter()
            .filter(|(l, _)| l.instrument() == instrument)
            .map(|(_, s)| match s {
                Series::Counter(v) => *v,
                _ => 0,
            })
            .sum()
    }

    /// A gauge's last value.
    pub fn gauge(&self, instrument: Instrument, values: &[&str]) -> Option<f64> {
        match self.find(instrument, values) {
            Some(Series::Gauge(v)) => Some(v),
            _ => None,
        }
    }

    /// A histogram's observation count and sum.
    pub fn histogram(&self, instrument: Instrument, values: &[&str]) -> (u64, f64) {
        match self.find(instrument, values) {
            Some(Series::Histogram { count, sum, .. }) => (count, sum),
            _ => (0, 0.0),
        }
    }

    /// Number of distinct series recorded for `instrument`.
    pub fn series_count(&self, instrument: Instrument) -> usize {
        let series = self.series.lock().unwrap_or_else(|e| e.into_inner());
        series
            .keys()
            .filter(|l| l.instrument() == instrument)
            .count()
    }

    /// Every recorded `(labels, rendered value)`, for assertions.
    pub fn dump(&self) -> Vec<(Labels, String)> {
        let series = self.series.lock().unwrap_or_else(|e| e.into_inner());
        series.iter().map(|(l, s)| (*l, format!("{s:?}"))).collect()
    }
}

impl MetricRecorder for InMemoryRecorder {
    fn counter_add(&self, labels: &Labels, value: u64) {
        let mut series = self.series.lock().unwrap_or_else(|e| e.into_inner());
        let entry = series.entry(*labels).or_insert(Series::Counter(0));
        if let Series::Counter(v) = entry {
            *v = v.saturating_add(value);
        }
    }

    fn gauge_set(&self, labels: &Labels, value: f64) {
        let mut series = self.series.lock().unwrap_or_else(|e| e.into_inner());
        series.insert(*labels, Series::Gauge(value));
    }

    fn histogram_record(&self, labels: &Labels, seconds: f64) {
        let mut series = self.series.lock().unwrap_or_else(|e| e.into_inner());
        let entry = series.entry(*labels).or_insert(Series::Histogram {
            buckets: [0; HISTOGRAM_BUCKETS_SECONDS.len() + 1],
            count: 0,
            sum: 0.0,
        });
        if let Series::Histogram {
            buckets,
            count,
            sum,
        } = entry
        {
            let i = HISTOGRAM_BUCKETS_SECONDS
                .iter()
                .position(|b| seconds <= *b)
                .unwrap_or(HISTOGRAM_BUCKETS_SECONDS.len());
            buckets[i] += 1;
            *count += 1;
            *sum += seconds;
        }
    }
}

/// One record buffered by a [`BridgeRecorder`].
#[derive(Clone, Debug, PartialEq)]
pub enum BridgeRecord {
    /// A counter increment.
    Counter(Labels, u64),
    /// A gauge value.
    Gauge(Labels, f64),
    /// A histogram observation, in seconds.
    Histogram(Labels, f64),
}

/// A bounded, non-blocking export bridge (`B-DIAG-03`).
///
/// SDK record calls enqueue into a fixed-capacity buffer and return
/// immediately. A host exporter drains it at its own pace with
/// [`Self::drain`]. When the buffer is full the record is dropped and
/// counted in [`Self::dropped`], the local equivalent of
/// `manja_telemetry_dropped_records_total`; a drop is never retried and never
/// reported through the bridge itself, so it cannot recurse.
pub struct BridgeRecorder {
    queue: Mutex<VecDeque<BridgeRecord>>,
    capacity: usize,
    dropped: AtomicU64,
}

impl BridgeRecorder {
    /// Default capacity, 1 024 records.
    pub const DEFAULT_CAPACITY: usize = 1024;
    /// Permitted capacity range.
    pub const CAPACITY_RANGE: (usize, usize) = (1, 65_536);

    /// A bridge buffering at most `capacity` records.
    pub fn new(capacity: usize) -> Result<Self, ObsConfigError> {
        let (min, max) = Self::CAPACITY_RANGE;
        if !(min..=max).contains(&capacity) {
            return Err(ObsConfigError::Capacity { min, max });
        }
        Ok(Self {
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            dropped: AtomicU64::new(0),
        })
    }

    /// Take every buffered record.
    pub fn drain(&self) -> Vec<BridgeRecord> {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.drain(..).collect()
    }

    /// Records dropped because the buffer was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Records currently buffered.
    pub fn len(&self) -> usize {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn push(&self, record: BridgeRecord) {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= self.capacity {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        } else {
            q.push_back(record);
        }
    }
}

impl MetricRecorder for BridgeRecorder {
    fn counter_add(&self, labels: &Labels, value: u64) {
        self.push(BridgeRecord::Counter(*labels, value));
    }

    fn gauge_set(&self, labels: &Labels, value: f64) {
        self.push(BridgeRecord::Gauge(*labels, value));
    }

    fn histogram_record(&self, labels: &Labels, seconds: f64) {
        self.push(BridgeRecord::Histogram(*labels, seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Counts allocations made by the current thread, so the disabled-path
    // test is unaffected by other tests running in parallel.
    mod counting {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;

        thread_local!(static ALLOCS: Cell<usize> = const { Cell::new(0) });

        pub struct Counting;

        unsafe impl GlobalAlloc for Counting {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
                unsafe { System.alloc(layout) }
            }
            unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                unsafe { System.dealloc(ptr, layout) }
            }
        }

        pub fn allocations() -> usize {
            ALLOCS.with(|c| c.get())
        }
    }

    #[global_allocator]
    static ALLOCATOR: counting::Counting = counting::Counting;

    #[test]
    fn disabled_recording_allocates_nothing() {
        let obs = Observability::disabled();
        let before = counting::allocations();
        for _ in 0..10_000 {
            obs.counter(
                Labels::http_operation(
                    Instrument::HttpOperationsTotal,
                    Method::Get,
                    Endpoint::Quote,
                    QuotaClass::Read,
                    HttpOperationResult::Ok,
                ),
                1,
            );
            obs.histogram(
                Labels::decode_duration(SourceMode::Live, PayloadKindLabel::Binary),
                0.001,
            );
        }
        assert_eq!(counting::allocations() - before, 0);
    }

    #[test]
    fn disabled_scope_records_nothing_and_needs_no_runtime() {
        let obs = Observability::disabled();
        obs.counter(Labels::auth_rejection(TransportLabel::Http), 1);
        obs.histogram(Labels::restore(RestoreResult::Sent), 0.1);
        let g = obs.gauge(Labels::quota(Instrument::HttpInFlight, QuotaClass::Read));
        g.set(3);
        // Typed diagnostics still work without a recorder.
        assert_eq!(
            obs.gauge_value(&Labels::quota(Instrument::HttpInFlight, QuotaClass::Read)),
            3.0
        );
    }

    #[test]
    fn clones_share_a_scope_and_gauges_settle_on_drop() {
        let rec = Arc::new(InMemoryRecorder::new());
        let obs = Observability::with_recorder(rec.clone());
        let clone = obs.clone();
        assert!(obs.same_scope(&clone));
        assert!(!obs.same_scope(&Observability::with_recorder(rec.clone())));
        let labels = Labels::quota(Instrument::HttpInFlight, QuotaClass::Mut);
        let a = obs.gauge(labels);
        let b = clone.gauge(labels);
        a.add(1);
        b.add(2);
        assert_eq!(rec.gauge(Instrument::HttpInFlight, &["mut"]), Some(3.0));
        drop(a);
        assert_eq!(rec.gauge(Instrument::HttpInFlight, &["mut"]), Some(2.0));
        drop(b);
        assert_eq!(rec.gauge(Instrument::HttpInFlight, &["mut"]), Some(0.0));
        assert_eq!(obs.gauge_value(&labels), 0.0);
    }

    #[test]
    fn oldest_age_is_the_maximum_computed_at_collection() {
        let obs = Observability::disabled();
        let a = obs.age(QueueRole::RawPrimary);
        let b = obs.age(QueueRole::RawPrimary);
        let long_ago = Instant::now() - std::time::Duration::from_secs(5);
        a.set_oldest(Some(long_ago));
        b.set_oldest(Some(Instant::now()));
        let labels = Labels::queue(Instrument::SdkQueueOldestAge, QueueRole::RawPrimary);
        let first = obs.gauge_value(&labels);
        assert!(first >= 5.0);
        let second = obs.gauge_value(&labels);
        assert!(second >= first, "ages advance at inspection");
        drop(a);
        assert!(obs.gauge_value(&labels) < 5.0);
        drop(b);
        assert_eq!(obs.gauge_value(&labels), 0.0);
    }

    #[test]
    fn bridge_is_bounded_and_counts_drops() {
        let bridge = Arc::new(BridgeRecorder::new(2).unwrap());
        let obs = Observability::with_recorder(bridge.clone());
        for _ in 0..5 {
            obs.counter(Labels::auth_rejection(TransportLabel::Ticker), 1);
        }
        assert_eq!(bridge.len(), 2);
        assert_eq!(bridge.dropped(), 3);
        assert_eq!(bridge.drain().len(), 2);
        assert!(bridge.is_empty());
        assert!(BridgeRecorder::new(0).is_err());
        assert!(BridgeRecorder::new(65_537).is_err());
    }

    #[test]
    fn static_dimensions_are_bounded() {
        let b = Observability::builder()
            .static_dimension("client", "primary")
            .unwrap();
        assert!(Observability::builder()
            .static_dimension("bad key", "v")
            .is_err());
        let mut b = b;
        for i in 0..3 {
            b = b.static_dimension(format!("k{i}"), "v").unwrap();
        }
        assert!(matches!(
            b.static_dimension("k9", "v"),
            Err(ObsConfigError::TooManyDimensions)
        ));
    }

    #[test]
    fn histograms_are_bucketed_not_accumulated() {
        let rec = InMemoryRecorder::new();
        let labels = Labels::shutdown(ShutdownResult::Clean);
        for _ in 0..10_000 {
            rec.histogram_record(&labels, 0.2);
        }
        assert_eq!(
            rec.histogram(Instrument::TickerShutdownDuration, &["clean"])
                .0,
            10_000
        );
        assert_eq!(rec.series_count(Instrument::TickerShutdownDuration), 1);
    }
}
