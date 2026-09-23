# How manja is verified

Every test is hermetic. HTTP and WebSocket tests run against loopback servers on
`127.0.0.1` (`tests/support/http.rs`, `tests/support/ws.rs`), and nothing contacts a
Kite endpoint. This document lists the fixtures, the coverage rows that tests cite by
their `INV-*` IDs, what each test target checks, and how the decoder and observability
are qualified. The behavior being checked is in [`contract.md`](contract.md).

---

## 1. Fixtures

### 1.1 Official samples

`kiteconnect-mocks/` is a Git submodule of
[zerodha/kiteconnect-mocks](https://github.com/zerodha/kiteconnect-mocks), pinned at
`c7a81238f93b4057c07257823b1bfaf30cba9ae5`. Its files are served byte for byte and are
never edited. Expected values are written out by hand from the documentation and the
fixture, never obtained by parsing the fixture a second time. The samples are upstream
examples, not a protocol specification. CI checks out the submodule before any test
runs, so a missing fixture fails setup instead of skipping tests.

### 1.2 Vendored ticker corpus

`tests/fixtures/ticker/` holds binary ticker messages that the official samples do not
provide: framed batches, index packets, the heartbeat, malformed cases, and one real
capture of 19 messages. [`MANIFEST.toml`](../tests/fixtures/ticker/MANIFEST.toml)
records each file's origin, source commit, SHA-256, size and class:

- **captured**: bytes recorded from a live Kite WebSocket session;
- **derived**: bytes cut, re-framed or generated from captured bytes;
- **synthetic**: hand-constructed bytes with no capture origin.

`tests/fixture_manifest.rs` re-hashes every file and rejects any file the manifest does
not list. Each `<case>.json` file holds the output of an independent decoder, called
qdx, for its `<case>.bin`. Those outputs are a cross-check, not protocol truth: the
protocol reference is `kite:websocket.md`. Where `manja` deliberately differs from qdx,
the difference is documented in `kite::decoder::framing` and `kite::decoder::packets`.

The capture file uses a test-only container read by `tests/support/capture.rs`. It is
not a `manja` format: each record is a little-endian `u64` receive time in Unix
nanoseconds, a little-endian `u32` length, and that many bytes of the unchanged Kite
message.

### 1.3 Supplemental fixtures

A fault or variant that no official file provides (an error envelope, a 429, an
oversized body, a new enum value, a malformed message) is written inline in the test and
labelled "Supplemental", with the official file it was derived from and what changed.

---

## 2. Coverage inventory

These rows name what the decoder must handle and where its evidence comes from. Their
IDs are stable. Tests cite the rows they cover, singly or as a range such as
`INV-F-02..05`; the gap rows are documentation.

### 2.1 Binary packet families (`kite:websocket.md:87-163`)

| ID | Family | Evidence |
|---|---|---|
| `INV-B-01` | LTP, 8 bytes | qdx `single_ltp` (synthetic); no official LTP packet exists (`INV-GAP-05`) |
| `INV-B-02` | Quote, 44 bytes | official `ticker_quote.packet`; qdx `single_quote` |
| `INV-B-03` | Full with depth, 184 bytes | official `ticker_full.packet`; qdx `single_full`; the capture |
| `INV-B-04` | Index quote, 28 bytes | qdx `single_index_quote` (synthetic) |
| `INV-B-05` | Index full, 32 bytes | qdx `single_index_full`; the capture |
| `INV-B-06` | Heartbeat, 1 byte: a message, not a tick | qdx `heartbeat` (synthetic) |

### 2.2 Framing cases (`kite:websocket.md:73-85`)

| ID | Case | Evidence |
|---|---|---|
| `INV-F-01` | Count plus length-prefixed batch | qdx `multi_packet`; all 19 capture records (2 692 packets) |
| `INV-F-02` | Count and length disagree | qdx `malformed_count` |
| `INV-F-03` | Truncated packet or header | qdx `truncated` |
| `INV-F-04` | Trailing bytes | qdx `trailing_bytes` |
| `INV-F-05` | Unknown packet length | qdx `unknown_size` |

### 2.3 Text messages (`kite:websocket.md:165-184`)

| ID | Message | Evidence |
|---|---|---|
| `INV-T-01` | `error` and `message` | supplemental |
| `INV-T-02` | `order` update | official `postback.json` as the `data` field |
| `INV-T-03` | Unknown `type` | supplemental |

### 2.4 Price scales (`kite:websocket.md:89`)

| ID | Segment | Expected |
|---|---|---|
| `INV-S-01` | CDS | divide by 10 000 000 |
| `INV-S-02` | BCD | refused (`contract.md` §5) |

### 2.5 Known gaps

| ID | Gap | How it is covered |
|---|---|---|
| `INV-GAP-05` | No official LTP packet (`ticker_ltp.json` has no `.packet` file) | the synthetic qdx `single_ltp` and generated packets |
| `INV-GAP-06` | No official index, heartbeat or broker-text messages | the vendored corpus for index and heartbeat; supplemental text messages |
| `INV-GAP-07` | No official non-2xx error envelopes (400, 403, 429, 500, a missing `error_type`, an HTML body) | supplemental envelopes in the documented shape (`kite:response-structure.md:17-28`, `kite:exceptions.md:7-16`), each stating the official body it was derived from |

---

## 3. Test targets

| Target | Features | Checks |
|---|---|---|
| `support` | `http`, `ticker` | the loopback harnesses and every fault they can inject |
| `protocol_common` | none | the always-available protocol types |
| `envelope_capture` | none | raw-observation envelopes over the capture keep bytes and receive times |
| `fixture_manifest` | none | the vendored corpus against `MANIFEST.toml` |
| `obs_schema` | none | the observability catalogue against `tests/obs_schema/catalogue.v1.txt` |
| `references` | none | every citation and ID in the repository resolves (§7) |
| `http_classification` | `http` | total success and error classification (`contract.md` §2.4) |
| `http_admission` | `http` | admission: a refused request fails before transport, as `NotStarted` |
| `http_scheduling` | `http` | deadlines, retries, one-attempt operations and cancellation evidence |
| `http_reads` | `http` | read response types against the official samples |
| `http_orders` | `http` | placement, modification, cancellation and conversion requests on the wire |
| `http_margins` | `http` | order margins, basket margins and order charges |
| `http_gtt` | `http` | GTT placement (single and two-leg), modification and deletion on the wire, one attempt each; the trigger list and trigger decoded from the official samples; and a fetched trigger converted back into a request (`contract.md` §2.12); the sandbox has no GTT (`kite:sandbox.md:298`), so these samples are its only evidence |
| `http_market` | `http` | quote completeness and the instrument master |
| `http_session` | `http` | token exchange and invalidation (`contract.md` §2.2) |
| `http_observability` | `http` | HTTP spans, metrics and diagnostics |
| `ticker_owner` | `ticker` | the single-owner ticker and its raw delivery |
| `ticker_subscriptions` | `ticker` | desired subscriptions, revisions and wire order |
| `ticker_lifecycle` | `ticker` | bounded reconnect, liveness and credential rejection |
| `ticker_delivery` | `ticker` | bounded delivery, cancellation and teardown |
| `ticker_status` | `ticker` | ticker spans, metrics and status |
| `decoder_framing` | `decoder` | framing against the vendored corpus |
| `decoder_packets` | `decoder` | every packet family, field by field |
| `decoder_text` | `decoder` | text messages without coercion |
| `decoder_adapter` | `decoder` | provenance and identical live and captured results |
| `decoder_qualification` | `decoder` | §4 |
| `observability_conformance` | all | §5 |
| `public_surface` | all | every public type is reachable through its documented path |

`ticker_lifecycle` runs on the real clock with the smallest permitted bounds. The
decoder targets need no async runtime. `tests/packaging/check.sh` packages the crate,
checks that no ignored, fixture, test, example or bench file ships, and builds and runs
an out-of-tree consumer against the package for the default, `http`, `ticker` and
`decoder` feature rows.

---

## 4. Decoder qualification

`tests/decoder/main.rs` (target `decoder_qualification`, feature `decoder` only) runs
every §2 row as a golden case, then a seeded property campaign. It uses an in-test
SplitMix64 generator, not a fuzzing crate. The default seed is `0x5eed2026`, and each
campaign XORs it with a fixed salt.

```text
cargo test --no-default-features --features decoder --release \
  --test decoder_qualification -- --nocapture
MANJA_FUZZ_SCALE=10 cargo test --no-default-features --features decoder --release \
  --test decoder_qualification
MANJA_FUZZ_SEED=0xdeadbeef cargo test --no-default-features --features decoder --release \
  --test decoder_qualification
```

| Campaign | Salt | Cases at scale 1 / 10 | Invariants |
|---|---|---|---|
| `campaign_count_and_length_framing` | `0x1` | 20 000 / 200 000 | a framed batch accounts for every byte; no frame is empty; the count is within `B-DEC-02`; every decoded packet re-encodes to its exact bytes |
| `campaign_trailing_bytes` | `0x2` | 10 000 / 100 000 | always `TrailingBytes`, with no packet exposed |
| `campaign_allocation_and_work_bounds` | `0x3` | 200 / 2 000, inputs up to 8 MiB | each call under 500 ms (linear work); over 16 MiB is `Oversized` before any work |
| `campaign_malformed_text` | `0x4` | 20 000 / 200 000, with mutations, truncation and nesting to 80 | success only for a JSON object with a string `type`, as the matching variant; detail ≤ 512 bytes |
| `campaign_numeric_extremes_and_segments` | `0x5` | 20 000 / 200 000 packets and prices × 9 segments | packets are byte-faithful; only a negative unsigned field is refused; every `i32` scales exactly in every supported segment; only BCD is refused |
| `campaign_capture_mutations` | `0x6` | 3 000 / 30 000 bit flips, truncations, insertions and extensions of the capture | the same framing and faithfulness invariants |

A further test decodes all 19 capture records with different receive times and after a
serialize-and-read-back round trip, and checks that the results are identical and that
a failed decode leaves the raw bytes readable.

**Limits.** The capture holds only 184-byte full and 32-byte index-full packets, so LTP,
quote, index quote, heartbeat and text coverage comes from the official samples, the
vendored corpus and supplemental messages. CDS scaling is exercised by golden values and
generated integers only. The campaign covers the parser and the adapter, not the live
socket path. A finite seeded campaign is evidence, not proof, for inputs it did not
generate.

---

## 5. Observability

### 5.1 Conformance scenarios

`tests/observability/main.rs` (target `observability_conformance`) and the
per-subsystem targets cover these scenarios:

| Scenario | Tests |
|---|---|
| Exact counts, durations and gauges | `http_observability` (one operation span and count per operation, one child per actual attempt, retries only when a new attempt starts); `ticker_status` (handshake, reconnect, command, restore and shutdown facts; one count per complete message); `decoder_adapter` (one decode count per observation) |
| Cancellation and teardown | gauges settle after success, error, cancellation and teardown, and aggregate across clones and owners |
| Asynchronous context | concurrent callers keep their own span context, including across the ticker mailbox |
| Cardinality | 10⁵ operations with distinct order IDs and quote keys, and 10⁵ distinct decoder source keys, stay within every series bound |
| Redaction | no seeded key, token, secret, checksum, order ID or symbol reaches any span, label, adapter record, diagnostic, status, error or `Debug` output |
| No consumer | a disabled, recording or saturated recorder changes no result, attempt count or delivery |
| Notification lag | a slow status reader sees coalesced revisions and delays nothing |
| Adapter failure | a stalled or never-drained `BridgeRecorder` changes no protocol outcome, and its drops are counted |
| Schema compatibility | the catalogue matches `tests/obs_schema/catalogue.v1.txt` |
| No per-message work | across 2 000 messages, the task count and span count stay constant |

Isolation from a recorder or subscriber callback that blocks or panics is not claimed
(`contract.md` §4.7).

### 5.2 Benchmark method

`benches/overhead.rs` measures four configurations: `disabled`
(`Observability::disabled()`, no subscriber), `metrics` (`InMemoryRecorder`),
`trace-info` (a capturing layer at INFO) and `trace-sampled` (the same layer at TRACE,
keeping 1 span in 10). The workloads are:

- **HTTP**: 100 sequential GETs, 64-way concurrent GETs, and one 429 followed by a
  success, against the loopback server serving the official `profile.json`.
- **Ticker**: full-mode messages of 1, 100 and 3 000 instruments from the vendored
  `single_full` packet, with a heartbeat after every tenth message.
- **Decoder**: every vendored binary message cycled to about 10⁶ messages, both through
  the pure parser and through the adapter.

Each case takes 30 samples after a discarded warm-up. Times are wall time per operation
on a current-thread runtime. Allocation and peak-heap figures are whole-process, so they
include the loopback harness. The benchmark uses a high-limit custom quota profile so
that it measures instrumentation rather than admission waits.

```text
MANJA_BUDGETS=check cargo bench --bench overhead
```

### 5.3 Budgets

With `MANJA_BUDGETS=check`, the benchmark exits non-zero when any budget is exceeded.
The budgets were measured and set on an Apple M5 (10 CPUs, 24 GiB, macOS 26.6.2) with
Rust 1.98.0, release profile and all features. Another host needs its own measurement.

| Budget | Limit | Measured worst |
|---|---|---|
| `metrics` against `disabled`, median time per operation | ≤ 1.25× + 0.5 µs | 1.17× |
| `trace-info` against `disabled` | ≤ 1.25× + 0.5 µs | 1.07× |
| `trace-sampled` against `disabled` | ≤ 1.35× + 1.5 µs | 1.07× |
| Decoder framing and decoding | 0 allocations per message | 0 |
| Extra allocations from recording | ≤ 1 per operation over `disabled` | 0 |
| Ticker delivery latency, p99.9 | ≤ 1 ms | 282 µs |
| Ticker peak retained heap | ≤ 16 MiB | 12.4 MiB |

---

## 6. Continuous integration

`.github/workflows/ci.yml` runs `cargo test`, doc tests included, for each of the eight
feature combinations on Rust 1.95.0 and 1.98.0, after checking that the
`kiteconnect-mocks` checkout matches its pin. For each row it also checks the resolved
dependency graph against `contract.md` §1. A separate job runs `cargo fmt`, Clippy with
warnings denied, and `cargo doc` with warnings denied, on 1.98.0.

---

## 7. Reference checks

`tests/references.rs` checks, without a network:

- every `kite:<page>.md:<lines>` citation names a page in
  [`kite-sources.toml`](kite-sources.toml) and stays within that page's lines, and each
  manifest entry has a URL, an access time, a SHA-256, a size and a line count;
- `QUOTA_PROFILE_VERSION` names the access date recorded for `exceptions.md`;
- every `B-HTTP-*`, `B-TK-*`, `B-DEC-*`, `B-DIAG-*`, `INV-*` and `BR-*` ID used in
  `src`, `tests`, `benches`, `examples`, `docs`, `scripts`, `.github`, `README.md`,
  `Cargo.toml` and the fixture manifest is defined in `docs/`;
- every `docs/<file>.md §N` reference names an existing section;
- no scanned file names material that is not part of the repository.

`scripts/verify-kite-sources.sh` needs a network: it fetches every listed page again and
reports any whose SHA-256, size or line count has changed. When a page changes, its
citations are re-checked against the new text and the manifest entry is updated.
