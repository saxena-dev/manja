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
| `protocol_common` | none | the always-available protocol types. The session expiry rule is covered by unit tests in `kite::protocol::datetime`: issued 05:59:59 IST expires the same day at 06:00, issued 06:00:00 IST the next day, a UTC input (01:00Z, 06:30 IST) the next day at 06:00 IST, sub-second inputs either side of 06:00, and for every input the result carries offset +05:30, falls at `SESSION_EXPIRY_IST`, and is strictly after the input by at most one day (`contract.md` §2.2) |
| `envelope_capture` | none | raw-observation envelopes over the capture keep bytes and receive times |
| `fixture_manifest` | none | the vendored corpus against `MANIFEST.toml` |
| `obs_schema` | none | the observability catalogue against `tests/obs_schema/catalogue.v1.txt` |
| `references` | none | every citation and ID in the repository resolves (§7) |
| `http_classification` | `http` | total success and error classification (`contract.md` §2.4) |
| `http_error_details` | `http` | no response value in an error detail: a seeded string, number and symbol in malformed envelopes, mistyped payloads and malformed instrument and mutual fund CSV rows never reach `detail()`, `Display` or `Debug`, and each detail still names what failed and where (`contract.md` §2.4) |
| `http_admission` | `http` | admission: a refused request fails before transport, as `NotStarted`. Priority lanes under a paused clock, through the public `Admission` API: with the default reserve, ten `Standard` acquisitions in the `Standard` class admit eight and the ninth is admitted exactly when the first ages out, while `Control` acquisitions admit at once in that state; eleven `Control` acquisitions in one second refuse the eleventh at once under a 1 ms wait bound and admit it exactly one second after the first; a reserve of 0 admits ten `Standard` at once; a placement reserve of 2 reserves 8 of 10 per second and 280 of 400 per minute and leaves the daily ceiling untouched; a reserve at a window's rate refuses `Standard` at once with `NoStandardCapacity`; `with_control_reserve` refuses 6 naming `B-HTTP-15`. Through the client, with a 61-per-minute `Standard` window and a reserve of 1: the second `Standard` read is refused before transport, a cancel and a `with_priority(Control)` read are sent, a cancel marked `Standard` is refused, and a reserve covering the class refuses a `Standard` read with an `Admission` error naming `B-HTTP-15` and sends nothing (`contract.md` §2.1, `contract.md` §3.2, `contract.md` §3.6). Headroom, paused clock: the default snapshot is 8 Standard and 10 Control with no next slot; exactly `standard_now` Standard calls admit and the next waits; at 0 the next slot is one second away, Control still has its two, and advancing the clock by the hint makes a Standard slot appear and admit at once; a dropped grant restores the number; reserve 0 reports equal lanes; a reserve at the window rate reports 0 Standard and no next slot; `day_remaining` is `None` outside placement and counts a ceiling of 3 down to `Some(0)`, where both lanes read 0 and the next placement is refused; `modifications_remaining` starts at 25, is 23 after two admissions for one order and 25 for another, a dropped grant does not count, and two `modify_order` calls through the client leave 23 (`contract.md` §3.6). Synchronous admission. Through the client on the real clock, with the clock paused only around the refused call: ten placements fill the window and the eleventh `place_order` is refused with zero paused-clock time elapsed (no timer awaited), kind `Admission`, stage `NotStarted`, cause `window_full`, a `retry_after` of at most one second that together with the time already spent covers the first grant's second, and nothing sent; after waiting the hint out the next placement is sent. Under the paused clock at the `Admission` level: `try_acquire` on a full window returns `Refused` with the instant the wait path would sleep to, counts no waiter, still reports a 1 ms hint at 999 ms and admits at the hint; on the Control lane the eleventh call in a second is refused with a one-second hint and reports 2 ms at 998 ms (the D7 hand-over). A read in the same scope with its lane full still waits and is sent after the slot frees (real clock); a ceiling of 2 refuses the third placement with `resets_at` 2026-09-24T00:00:00+05:30 under a fixed wall clock and no `retry_after`, with the clock paused around the refusal; the 26th modification of one order is `ModificationCeiling` with neither hint, at the `Admission` level and through the client (`contract.md` §2.4, `contract.md` §3.6) |
| `http_scheduling` | `http` | deadlines, retries, one-attempt operations and cancellation evidence |
| `http_reads` | `http` | read response types against the official samples; every response's `meta` carries HTTP status 200, attempt 1 and no message for the official profile and order book, with distinct increasing operation IDs across two calls, and `Response::map` keeps the meta (`contract.md` §2.18) |
| `http_order_rows` | `http` | strict and tolerant decoding of the order and trade lists: every official list the same under both policies; one row made invalid by a path-changing ID, a malformed timestamp or a malformed price, in the order, history, trade and order-trade lists, rejected in place or failing the strict list with only the count and first index; an array of non-objects as rejected rows; a strict rejection recorded as a failed operation and attempt and in the diagnostics; the rejected-row counter; envelope failures (malformed JSON, a non-object body, a missing or non-success status, missing, null or non-array data, and broker, HTTP and credential errors) identical on all four lists under both policies; and no seeded value in any detail or `Debug` output (`contract.md` §2.16); broker state unchanged: a row derived from `orders.json` with 3 of 10 filled round-trips `status`, `quantity`, `filled_quantity`, `pending_quantity` and `cancelled_quantity` under both policies, and the same row with the undocumented status `HALF FILLED` keeps it as `Inbound::Unknown` with its wire text (`contract.md` §2.18) |
| `http_order_rows_span` | `http` | a strict rejection ends its operation span as `decode_error`, and its attempt span records `error_class` `decode_error`. Like every span-capturing test, it installs its capture through `tests/support/spans.rs`, so a concurrent test without a subscriber cannot have the span cached as disabled |
| `http_orders` | `http` | placement, modification, cancellation and conversion requests on the wire; path-changing order IDs unable to be built; an order ID from the order book passed straight back into a modification; and a path-changing ID in a response decoded as an error. Wire forms: for placement, modification, cancellation (the empty body, SHA-256 `e3b0c4...`) and conversion, the captured request's method, path, content type and body equal the form's and `sha256(captured body) == wire.sha256()`; `wire()` is deterministic over 100 calls and across a clone, and differs for a different price; `Display` of a placement form equals the form body, `Debug` shows the same body and the hash, neither contains `api_key`, `access_token` or `Authorization`, and `WireForm`'s fields name no credential type (a check over `src/kite/connect/wire.rs`); an invalid request has no form (`contract.md` §2.1). Response meta: a supplemental receipt derived from `order_response.json` with an envelope-level `message` (no official sample carries one; `autoslice_response.json`'s sits inside a slice) round-trips it in `meta.message` with a populated operation ID, HTTP status 200 and attempt 1, and the official receipt carries `None` (`contract.md` §2.18) |
| `http_margins` | `http` | order margins, basket margins and order charges |
| `http_holdings_authorisation` | `http` | holdings authorisation: named ISIN and quantity pairs and the entire holdings on the wire, the request ID from the official `holdings_auth.json`, the documented portal URL and its encoding, a malformed ISIN sending nothing, one attempt after a lost response or a 429, and a 428 sell refusal reported by `requires_holdings_authorisation` (`contract.md` §2.15); the named and the entire-holdings requests each sent exactly as their wire form, hash for hash, the latter with no body (`contract.md` §2.1) |
| `http_mutual_funds` | `http` | mutual fund orders, one order by UUID, SIPs, holdings and the instrument list decoded field by field from the official samples; an unsafe order ID unable to be built; a malformed CSV row; an unknown status; read retries; and the standard quota class (`contract.md` §2.14). The mutation fixtures (`mf_order_*`, `mf_sip_place`, `mf_sip_modify`, `mf_sip_cancel`) and `mf_sip_info.json` have no documented endpoint and are not used |
| `http_historical` | `http` | historical candles: every interval's path and the IST query on the wire, minute and open-interest candles decoded from the official samples, continuous data, a range that ends before it starts, a malformed candle, read retries, and the client admitting them in their own 3-per-second class, apart from quotes (`contract.md` §2.13); a paused-clock unit test in `kite::connect::admission` covers the class's timing |
| `http_gtt` | `http` | GTT placement (single and two-leg), modification and deletion on the wire, one attempt each; each mutation dispatched with a permit, and a mismatched, expired or foreign permit or an invalid request starting no transport; the trigger list and trigger decoded from the official samples; and a fetched trigger converted back into a request (`contract.md` §2.12); a two-leg placement, a modification and a deletion each sent exactly as their wire form, hash for hash, with the JSON `condition` and `orders` fields inside the form body and the forms deterministic over 100 calls (`contract.md` §2.1); the sandbox has no GTT (`kite:sandbox.md:298`), so these samples are its only evidence |
| `http_market` | `http` | quote completeness and the instrument master |
| `http_session` | `http` | token exchange and invalidation (`contract.md` §2.2); each sent exactly as its wire form, hash for hash, with the exchange form's `Display` and `Debug` redacting the request token and checksum and the invalidation form's `Debug` redacting the access token while neither renders the secret (`contract.md` §2.1) |
| `http_observability` | `http` | HTTP spans, metrics and diagnostics; the operation span's `priority` field is `standard` on a read, `control` on a cancel, a GTT deletion and a `with_priority(Control)` read, and absent from the admission span (`contract.md` §4.2); after a 503 then a 200, the response's `meta.attempt` is 2 and its `meta.operation_id` equals the operation span's `operation_id` (`contract.md` §2.18); a placement refused for a full window ends its operation span with `result=admission_rejected`, `stage=not_started` and `admission_cause=window_full`, its child admission span records `admission_result=refused`, and the admission-wait histogram counts one `refused` sample in the `mut` class (`contract.md` §4.2) |
| `ticker_owner` | `ticker` | the single-owner ticker and its raw delivery |
| `ticker_subscriptions` | `ticker` | desired subscriptions, revisions and wire order |
| `ticker_lifecycle` | `ticker` | bounded reconnect, liveness and credential rejection |
| `ticker_delivery` | `ticker` | bounded delivery, cancellation and teardown |
| `ticker_status` | `ticker` | ticker spans, metrics and status |
| `decoder_framing` | `decoder` | framing against the vendored corpus |
| `decoder_packets` | `decoder` | every packet family, field by field |
| `decoder_text` | `decoder` | text messages without coercion, and a failed parse whose detail names what failed but no value from the message (`contract.md` §2.9) |
| `decoder_adapter` | `decoder` | provenance and identical live and captured results, and a retained text diagnostic that holds no value from the message (`contract.md` §2.9) |
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
feature configurations on Rust 1.88.0, the minimum supported version, and on the stable
channel, after checking that the `kiteconnect-mocks` checkout matches its pin. For each
row it also checks the resolved dependency graph against `contract.md` §1. A separate job
runs `cargo fmt`, Clippy with warnings denied, and `cargo doc` with warnings denied, on
stable.

The doc tests include every Rust example in `README.md`: `src/lib.rs` includes the README
under `cfg(doctest)`, so its examples compile on every row with all three features and
cannot drift from the API.

---

## 7. Reference checks

`tests/references.rs` checks, without a network:

- every `kite:<page>.md:<lines>` citation names a page in
  [`kite-sources.toml`](kite-sources.toml) and stays within that page's lines, and each
  manifest entry has a URL, an access time, a SHA-256, a size and a line count;
- `QUOTA_PROFILE_VERSION` names the access date recorded for `exceptions.md`, with an
  optional `+r<revision>`;
- every `B-HTTP-*`, `B-TK-*`, `B-DEC-*`, `B-DIAG-*`, `INV-*` and `BR-*` ID used in
  `src`, `tests`, `benches`, `examples`, `docs`, `scripts`, `.github`, `README.md`,
  `Cargo.toml` and the fixture manifest is defined in `docs/`;
- every `docs/<file>.md §N` reference names an existing section;
- no scanned file names material that is not part of the repository.

`scripts/verify-kite-sources.sh` needs a network: it fetches every listed page again and
reports any whose SHA-256, size or line count has changed. When a page changes, its
citations are re-checked against the new text and the manifest entry is updated.
