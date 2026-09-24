//! Observability overhead (`docs/verification.md` §5.2).
//!
//! `cargo bench --offline --bench overhead`. Release profile; each case runs
//! a warm-up sample that is discarded, then 30 measured samples. Four
//! configurations: (1) `disabled`: `Observability::disabled()` and no
//! subscriber; (2) `metrics`: an `InMemoryRecorder`; (3) `trace-info`: a
//! capturing subscriber filtered at INFO; (4) `trace-sampled`: a capturing
//! subscriber at TRACE that keeps one span in ten. Workloads use the loopback
//! harnesses and the committed fixtures only.
//!
//! Measures per operation: wall time on a current-thread runtime (median
//! and p99 across samples), allocations (counting allocator, whole process,
//! harness included), and peak retained heap during the case. Per-message
//! latency percentiles are reported for the ticker (receive to consumer)
//! and the decoder.
//!
//! With `MANJA_BUDGETS=check` the run fails when a measured delta against
//! `disabled` exceeds its budget (see `BUDGETS`, `docs/verification.md` §5.3).

#[path = "../tests/support/mod.rs"]
mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use manja::kite::connect::admission::{
    Admission, AdmissionLimits, QuotaProfile, RateClass, Window,
};
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::decoder::adapter::Adapter;
use manja::kite::decoder::framing::{frame, FramingLimits, Message as Framed};
use manja::kite::decoder::packets::decode;
use manja::kite::envelope::{
    MonotonicElapsed, PayloadKind, RawObservation, ReceiveTime, SourceIdentity, SourceSequencer,
};
use manja::kite::obs::schema::SourceMode;
use manja::kite::obs::{InMemoryRecorder, Observability};
use manja::kite::protocol::InstrumentToken;
use manja::kite::ticker::actor::owner::{TickerBuilder, TickerEvent, TickerLimits};
use manja::kite::ticker::Mode;
use tokio_tungstenite::tungstenite::Message;
use tracing::span::{Attributes, Id};
use tracing::Subscriber;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

// ---- counting allocator ------------------------------------------------

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// Counts, then forwards to `System` with the caller's arguments unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(live, Ordering::Relaxed);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract for `layout`.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller passes a block this allocator returned for `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// ---- configurations -----------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode4 {
    Disabled,
    Metrics,
    TraceInfo,
    TraceSampled,
}

const MODES: [Mode4; 4] = [
    Mode4::Disabled,
    Mode4::Metrics,
    Mode4::TraceInfo,
    Mode4::TraceSampled,
];

impl Mode4 {
    fn name(self) -> &'static str {
        match self {
            Mode4::Disabled => "disabled",
            Mode4::Metrics => "metrics",
            Mode4::TraceInfo => "trace-info",
            Mode4::TraceSampled => "trace-sampled",
        }
    }
    fn obs(self) -> Observability {
        match self {
            Mode4::Metrics => Observability::with_recorder(Arc::new(InMemoryRecorder::new())),
            _ => Observability::disabled(),
        }
    }
    fn subscriber(self) -> Option<tracing::subscriber::DefaultGuard> {
        let level = match self {
            Mode4::TraceInfo => LevelFilter::INFO,
            Mode4::TraceSampled => LevelFilter::TRACE,
            _ => return None,
        };
        let layer = Sampler {
            every: if self == Mode4::TraceSampled { 10 } else { 1 },
            seen: Arc::new(AtomicU64::new(0)),
            kept: Arc::new(AtomicU64::new(0)),
        };
        Some(tracing::subscriber::set_default(
            tracing_subscriber::registry().with(layer.with_filter(level)),
        ))
    }
}

// Keeps one span in `every` (a fixed ratio recorded with the results).
struct Sampler {
    every: u64,
    seen: Arc<AtomicU64>,
    kept: Arc<AtomicU64>,
}

impl<S: Subscriber> Layer<S> for Sampler {
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _: Context<'_, S>) {
        if self
            .seen
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(self.every)
        {
            let _name = attrs.metadata().name();
            self.kept.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ---- measurement ------------------------------------------------------

const SAMPLES: usize = 30;

struct Stats {
    ns_per_op: Vec<f64>,
    allocs_per_op: f64,
    peak_bytes: usize,
    latencies_ns: Vec<u64>,
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i]
}

fn pct_u(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

struct Row {
    case: String,
    mode: Mode4,
    median_ns: f64,
    p99_ns: f64,
    allocs: f64,
    peak_kib: usize,
    lat: Option<(u64, u64, u64)>,
}

async fn measure<F, Fut>(case: &str, mode: Mode4, ops_per_sample: u64, mut sample: F) -> Row
where
    F: FnMut(Mode4, Observability) -> Fut,
    Fut: std::future::Future<Output = Vec<u64>>,
{
    let _guard = mode.subscriber();
    // One recording scope per case, as a host would hold one.
    let obs = mode.obs();
    // Warm-up, discarded.
    let _ = sample(mode, obs.clone()).await;
    let mut stats = Stats {
        ns_per_op: Vec::new(),
        allocs_per_op: 0.0,
        peak_bytes: 0,
        latencies_ns: Vec::new(),
    };
    let base_live = LIVE.load(Ordering::Relaxed);
    PEAK.store(base_live, Ordering::Relaxed);
    let allocs_before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let lat = sample(mode, obs.clone()).await;
        stats
            .ns_per_op
            .push(start.elapsed().as_nanos() as f64 / ops_per_sample as f64);
        stats.latencies_ns.extend(lat);
    }
    stats.allocs_per_op = (ALLOCS.load(Ordering::Relaxed) - allocs_before) as f64
        / (ops_per_sample as f64 * SAMPLES as f64);
    stats.peak_bytes = PEAK.load(Ordering::Relaxed).saturating_sub(base_live);
    stats.ns_per_op.sort_by(|a, b| a.partial_cmp(b).unwrap());
    stats.latencies_ns.sort_unstable();
    let lat = (!stats.latencies_ns.is_empty()).then(|| {
        (
            pct_u(&stats.latencies_ns, 0.50),
            pct_u(&stats.latencies_ns, 0.99),
            pct_u(&stats.latencies_ns, 0.999),
        )
    });
    Row {
        case: case.to_string(),
        mode,
        median_ns: pct(&stats.ns_per_op, 0.5),
        p99_ns: pct(&stats.ns_per_op, 0.99),
        allocs: stats.allocs_per_op,
        peak_kib: stats.peak_bytes / 1024,
        lat,
    }
}

// ---- workloads --------------------------------------------------------

fn client(base: &str, obs: Observability) -> HTTPClient {
    let limits = HttpLimits::default().with_scheduler(
        SchedulerLimits::default()
            .with_backoff(Duration::from_millis(10), Duration::from_millis(10))
            .unwrap()
            .with_jitter_seed(7),
    );
    // A high standard-class quota: this measures instrumentation overhead,
    // not the documented 10 per second admission wait.
    let fast = Window::new(1_000_000, Duration::from_secs(1));
    let mut profile = QuotaProfile::kite_v3().with_version("bench");
    for class in [
        RateClass::Quote,
        RateClass::Historical,
        RateClass::OrderModification,
        RateClass::Standard,
    ] {
        profile = profile.with_windows(class, vec![fast]).unwrap();
    }
    HTTPClient::builder(Config::new(base).with_limits(limits))
        .admission(Admission::new(profile, AdmissionLimits::default()))
        .observability(obs)
        .credentials(Credentials::new("k", "t").unwrap())
        .build()
        .unwrap()
}

fn profile() -> support::http::Reply {
    support::http::Reply::json(support::fixtures::json_body("profile.json").unwrap())
}

async fn http_sequential(obs: Observability, n: usize) -> Vec<u64> {
    let h = support::http::HttpHarness::start((0..n).map(|_| profile()).collect()).await;
    let c = client(&h.base_url(), obs);
    for _ in 0..n {
        c.user().profile().await.unwrap();
    }
    vec![]
}

async fn http_concurrent(obs: Observability, n: usize) -> Vec<u64> {
    let h = support::http::HttpHarness::start((0..n).map(|_| profile()).collect()).await;
    let c = client(&h.base_url(), obs);
    let user = c.user();
    let calls: Vec<_> = (0..n).map(|_| user.profile()).collect();
    for r in futures_util::future::join_all(calls).await {
        r.unwrap();
    }
    vec![]
}

async fn http_retry(obs: Observability) -> Vec<u64> {
    let busy = support::http::Reply::Respond {
        status: 429,
        content_type: "application/json",
        body:
            br#"{"status":"error","message":"Too many requests","error_type":"NetworkException"}"#
                .to_vec(),
    };
    let h = support::http::HttpHarness::start(vec![busy, profile()]).await;
    client(&h.base_url(), obs).user().profile().await.unwrap();
    vec![]
}

fn full_message(instruments: usize) -> Vec<u8> {
    let packet = support::capture::read_ticker_fixture("protocol/single_full.bin")[4..].to_vec();
    let mut m = (instruments as u16).to_be_bytes().to_vec();
    for _ in 0..instruments {
        m.extend(184u16.to_be_bytes());
        m.extend(&packet);
    }
    m
}

async fn ticker(obs: Observability, instruments: usize, messages: usize) -> Vec<u64> {
    use support::ws::{Handshake, Step, WsConnection, WsHarness};
    let msg = full_message(instruments);
    let mut steps = Vec::with_capacity(messages + messages / 10);
    for i in 0..messages {
        steps.push(Step::Send(Message::Binary(msg.clone())));
        if i % 10 == 9 {
            steps.push(Step::Send(Message::Binary(vec![0]))); // heartbeat
        }
    }
    let total = steps.len();
    let w = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps,
    }])
    .await;
    let (handle, mut events, guard) = TickerBuilder::new(Credentials::new("k", "t").unwrap())
        .url(w.url())
        .limits(TickerLimits::default())
        .observability(obs)
        .spawn()
        .unwrap();
    handle
        .subscribe([InstrumentToken::new(408065)], Mode::Full)
        .await
        .unwrap();
    let mut lat = Vec::with_capacity(total);
    let mut raw = 0;
    while raw < total {
        if let Some(Ok(TickerEvent::Raw(r))) = events.next().await {
            raw += 1;
            let now = ReceiveTime::now().unix_nanos();
            lat.push((now - r.received_at().unix_nanos()).max(0) as u64);
        }
    }
    handle.shutdown().await.unwrap();
    while events.next().await.is_some() {}
    guard.join().await;
    lat
}

// Every committed binary message: the protocol fixtures and the capture.
fn corpus() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for name in [
        "heartbeat",
        "single_ltp",
        "single_quote",
        "single_full",
        "single_index_quote",
        "single_index_full",
        "multi_packet",
    ] {
        out.push(support::capture::read_ticker_fixture(&format!(
            "protocol/{name}.bin"
        )));
    }
    let capture = support::capture::read_real_capture();
    for r in support::capture::read_capture(&capture).unwrap() {
        out.push(r.payload.to_vec());
    }
    out
}

fn decode_standalone(corpus: &[Vec<u8>], messages: usize) -> Vec<u64> {
    let mut lat = Vec::with_capacity(messages);
    for i in 0..messages {
        let m = &corpus[i % corpus.len()];
        let start = Instant::now();
        if let Ok(Framed::Packets(frames)) = frame(m, FramingLimits::default()) {
            for f in frames.iter() {
                std::hint::black_box(decode(&f).ok());
            }
        }
        lat.push(start.elapsed().as_nanos() as u64);
    }
    lat
}

// Allocations made by framing and decoding alone, over `rounds` passes of
// the corpus. Nothing else runs inside the counted region.
fn decode_allocations(corpus: &[Vec<u8>], rounds: usize) -> u64 {
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..rounds {
        for m in corpus {
            if let Ok(Framed::Packets(frames)) = frame(m, FramingLimits::default()) {
                for f in frames.iter() {
                    std::hint::black_box(decode(&f).ok());
                }
            }
        }
    }
    ALLOCS.load(Ordering::Relaxed) - before
}

fn decode_replay(
    mode: Mode4,
    obs: &Observability,
    observations: &[RawObservation],
    messages: usize,
) -> Vec<u64> {
    let adapter = Adapter::new(SourceMode::Replay, obs).with_spans(mode == Mode4::TraceSampled);
    let mut lat = Vec::with_capacity(messages);
    for i in 0..messages {
        let start = Instant::now();
        std::hint::black_box(
            adapter
                .decode(&observations[i % observations.len()])
                .unwrap(),
        );
        lat.push(start.elapsed().as_nanos() as u64);
    }
    lat
}

// ---- budgets ----------------------------------------------------------

// Budgets set against the recorded host (`docs/verification.md` §5.3):
// the maximum median time per operation of each configuration over
// `disabled` for the same case, as a ratio plus an absolute allowance.
const BUDGETS: &[(&str, f64, f64)] = &[
    // (mode, max ratio, absolute allowance ns)
    ("metrics", 1.25, 500.0),
    ("trace-info", 1.25, 500.0),
    ("trace-sampled", 1.35, 1500.0),
];
/// Recording adds at most this many allocations per operation.
const MAX_EXTRA_ALLOCS_PER_OP: f64 = 1.0;
/// Ticker delivery latency, p99.9, receive to consumer.
const MAX_TICKER_P999_NS: u64 = 1_000_000;
/// Peak retained heap of a ticker case.
const MAX_TICKER_PEAK_KIB: usize = 16 * 1024;

fn main() {
    // `cargo test --all-targets` runs this binary without `--bench`: check
    // that the decoder stays allocation-free and stop, rather than measure
    // an unoptimized build.
    if !std::env::args().any(|a| a == "--bench") {
        let allocs = decode_allocations(&corpus(), 1);
        assert_eq!(allocs, 0, "frame and decode allocate");
        println!("overhead: smoke run only; measure with `cargo bench`");
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let corpus = corpus();
    let mut seq = SourceSequencer::new(SourceIdentity::generate());
    seq.begin_epoch();
    let observations: Vec<RawObservation> = corpus
        .iter()
        .map(|m| {
            RawObservation::new(
                seq.next_key(),
                PayloadKind::Binary,
                ReceiveTime::from_unix_nanos(0),
                MonotonicElapsed::from_nanos(0),
                m.clone(),
                16 << 20,
            )
            .unwrap()
        })
        .collect();
    let decoder_messages = 1_000_000 / SAMPLES;

    let mut rows = Vec::new();
    rt.block_on(async {
        for mode in MODES {
            rows.push(
                measure("http sequential GET", mode, 100, |_, o| {
                    http_sequential(o, 100)
                })
                .await,
            );
            rows.push(measure("http 64-way GET", mode, 64, |_, o| http_concurrent(o, 64)).await);
            rows.push(measure("http 429 then ok", mode, 1, |_, o| http_retry(o)).await);
            for (inst, msgs) in [(1usize, 2000usize), (100, 500), (3000, 20)] {
                let total = msgs + msgs / 10;
                rows.push(
                    measure(
                        &format!("ticker {inst} instruments"),
                        mode,
                        total as u64,
                        |_, o| ticker(o, inst, msgs),
                    )
                    .await,
                );
            }
            let c = corpus.clone();
            rows.push(
                measure(
                    "decode standalone",
                    mode,
                    decoder_messages as u64,
                    |_, _| {
                        let c = c.clone();
                        async move { decode_standalone(&c, decoder_messages) }
                    },
                )
                .await,
            );
            let o = observations.clone();
            rows.push(
                measure(
                    "decode replay (adapter)",
                    mode,
                    decoder_messages as u64,
                    |m, obs| {
                        let o = o.clone();
                        async move { decode_replay(m, &obs, &o, decoder_messages) }
                    },
                )
                .await,
            );
        }
    });

    println!("| case | config | median ns/op | p99 ns/op | allocs/op | peak KiB | latency p50 / p99 / p99.9 ns | delta vs disabled |");
    println!("|---|---|---|---|---|---|---|---|");
    let mut failures = Vec::new();
    for r in &rows {
        let base = rows
            .iter()
            .find(|b| b.case == r.case && b.mode == Mode4::Disabled)
            .unwrap();
        let delta = r.median_ns / base.median_ns;
        let lat = r
            .lat
            .map_or("—".to_string(), |(a, b, c)| format!("{a} / {b} / {c}"));
        println!(
            "| {} | {} | {:.0} | {:.0} | {:.1} | {} | {} | {:.2}× |",
            r.case,
            r.mode.name(),
            r.median_ns,
            r.p99_ns,
            r.allocs,
            r.peak_kib,
            lat,
            delta
        );
        if let Some((_, ratio, allowance)) = BUDGETS.iter().find(|b| b.0 == r.mode.name()) {
            if r.median_ns > base.median_ns * ratio + allowance {
                failures.push(format!(
                    "{} {}: {:.2}× over {ratio}×",
                    r.case,
                    r.mode.name(),
                    delta
                ));
            }
            if r.allocs > base.allocs + MAX_EXTRA_ALLOCS_PER_OP {
                failures.push(format!(
                    "{} {}: {:.1} allocs/op",
                    r.case,
                    r.mode.name(),
                    r.allocs
                ));
            }
        }
        if r.case.starts_with("ticker") {
            if r.lat.is_some_and(|(_, _, p999)| p999 > MAX_TICKER_P999_NS) {
                failures.push(format!("{} {}: p99.9 over 1 ms", r.case, r.mode.name()));
            }
            if r.peak_kib > MAX_TICKER_PEAK_KIB {
                failures.push(format!(
                    "{} {}: {} KiB retained",
                    r.case,
                    r.mode.name(),
                    r.peak_kib
                ));
            }
        }
    }
    println!(
        "\nhost: {} cpus; samples {SAMPLES}; decoder messages per config {}; sampling 1 in 10 at TRACE",
        std::thread::available_parallelism().map_or(0, |n| n.get()),
        decoder_messages * SAMPLES
    );
    let decode_allocs = decode_allocations(&corpus, 1000);
    println!(
        "decode standalone: {decode_allocs} allocations over {} messages",
        1000 * corpus.len()
    );
    if decode_allocs > 0 {
        failures.push(format!("decode standalone allocates: {decode_allocs}"));
    }
    if std::env::var("MANJA_BUDGETS").as_deref() == Ok("check") {
        if failures.is_empty() {
            println!("budgets: pass");
        } else {
            for f in &failures {
                println!("budget exceeded: {f}");
            }
            std::process::exit(1);
        }
    }
}
