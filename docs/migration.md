# Migrating from manja 0.2 to 0.3

0.3.0 is a breaking release under Cargo's rules for 0.x versions. Every break ships in
it together: when every endpoint's return type changes, every caller migrates everything
at once, and a deprecation cycle for the removed permit API would have bought nothing.
There is no 0.2.1. This document lists the one behavioural break that compiles unchanged
(§2), every intentional break with its `BR-*` ID (§3), what was added (§4), what changed
while keeping its path (§5), and where each design decision of the release lands in the
contract, the tests and this guide (§6). The guide for the previous release, 0.1 to 0.2,
is kept in full in §7. The behavior you can rely on is in [`contract.md`](contract.md).

## 1. Compatibility policy

- **Paths.** Under the default features these keep their paths:
  `manja::kite::connect::client::HTTPClient`; the resources in
  `manja::kite::connect::api` (`Session`, `User`, `Orders`, `Portfolio`, `Market`,
  `Margins`, `Charges`, `MutualFunds`, `Gtt`); the model re-exports in
  `manja::kite::connect::models`; `manja::kite::ticker` and `manja::kite::decoder`. The
  ticker and decoder types are unchanged in 0.3, except that the packet error
  `PacketErrorKind::NegativeQuantity` is removed and `ScaleError::UnsupportedSegment` is
  deprecated; what the decoder reads from count fields, and the price scale of BCD and
  NCO, changed (`BR-20`, `BR-21`).
- **Features.** The default is `http`, `ticker` and `decoder`. A path disappears only
  when you disable default features. With no features, or with `decoder` alone, the
  crate has no Tokio or network dependency. `http` now enables Tokio's runtime handle
  (`tokio/rt`), because a mutation's attempt runs on a task the client spawns
  (`contract.md` §2.17).
- **Minimum Rust version.** 1.88.0, with edition 2024, unchanged.
- **Versioning.** Before 1.0, breaking changes ship only in minor-version bumps, each
  listed in this guide. Observable behaviour is the contract, so a change that compiles
  against old callers but behaves differently is a break too (§2). Two versioned
  constants move in 0.3.0: `DECODER_VERSION` 1 to 2 and `CONVERSION_POLICY_VERSION` 1
  to 2 (`BR-20`, `BR-21`). `ENVELOPE_VERSION` 1.0, `OBS_SCHEMA_VERSION` 1 (the new span
  fields are additive) and `QUOTA_PROFILE_VERSION` are unchanged (`contract.md` §6).
- **What a successful call means is unchanged.** An order receipt means the broker
  accepted the request, not that it filled; a ticker command's `Ok(Revision)` means the
  owner accepted it; nothing the SDK returns is a statement that data is fresh.

## 2. The break that compiles unchanged

`BR-16` is the one to read first, because the compiler will not find it for you. In
0.2 a mutation (an order placement, modification or cancellation, a position
conversion, a holdings authorisation, a GTT change or a session call) that found its
rate-limit window full waited up to five seconds (`B-HTTP-06`) for a slot. In 0.3 it
never waits: it is admitted at once or refused before transport with an `Admission`
error that says when to try again (`retry_after()`) or when the daily ceiling resets
(`resets_at()`). Code that compiled against 0.2 and relied on the wait now sees refusals
under load, and a refusal is affirmative evidence that nothing was sent, so the call is
safe to repeat once the hint has passed. Reads and the margin and charge calculations
still wait. The reason is in `contract.md` §5: a wait between a caller's decision and
the socket write makes that decision stale by construction. With `BR-17` the mutation
call is itself the admission boundary, so the permit API that used to reserve a slot
ahead of the call is gone, and `Admission::headroom` (§4) is the advisory, local view
of capacity for a caller that wants to look before it calls.

## 3. Intentional breaks

| ID | Change | What to do |
|---|---|---|
| `BR-15` | Every JSON endpoint returns `Response<T> { data: T, meta: ResponseMeta }` instead of `KiteApiResponse<T>`. `Response::map(f)` maps `data` and keeps `meta`. `data` is the value itself, never `Option`; the `status` and `error_type` fields are gone, since both were constant on every returned value; Kite's optional `message` moves to `meta.message`, next to the HTTP status, the operation ID and the attempt number (`contract.md` §2.18) | Drop the unwrap. Before: `let profile = client.user().profile().await?.data.expect("data");` and `let book = client.orders().list_orders().await?.data.unwrap_or_default();`. After: `let profile = client.user().profile().await?.data;` and `let book = client.orders().list_orders().await?.data;`. Code that read `response.message` reads `response.meta.message`; code that checked `response.status == "success"` deletes the check, which could never fail |
| `BR-16` | A mutation or session operation (placement, modification, cancellation, position conversion, holdings authorisation, GTT placement, modification and deletion, token exchange and invalidation) no longer waits for admission capacity. It is admitted at once when every window of its class has a free slot, and otherwise refused before transport with an `Admission` error (stage `NotStarted`) carrying `retry_after()` for a full window or `resets_at()` for the spent daily ceiling; the per-order modification limit refuses with cause `modification_ceiling`. Only reads and calculations still wait under `B-HTTP-06`. `AdmissionError::ModificationLimitReached` is renamed `ModificationCeiling`, `DailyCeilingReached` carries `resets_at`, and `Refused { retry_after }` is new (`contract.md` §2.4, `contract.md` §3.2, `contract.md` §3.6) | Code that relied on the SDK queueing a mutation for up to five seconds now sees refusals under load. Handle `HttpErrorKind::Admission` on every mutation: read `retry_after()` or `resets_at()`, or consult `Admission::headroom` before the call, and decide when to call again; nothing was sent, so the call is safe to repeat. Match the renamed variant |
| `BR-17` | The dispatch permit API is removed: `HTTPClient::admit`, `DispatchPermit`, `PermitTarget`, bound `B-HTTP-12` (`SchedulerLimits::with_permit_validity` and `permit_validity`) and every `*_with_permit` method. With `BR-16` the mutation call is itself the admission boundary: it crosses into transport now or returns affirmative `NotStarted` evidence with a hint. `B-HTTP-12` stays listed as retired and its ID is never reused (`contract.md` §3.2, `contract.md` §5) | Call the plain method, one per removed method, after your own checks; a refusal sent nothing, so repeat the call when `retry_after()` has passed or `Admission::headroom` shows a slot. Removed, with its replacement: `place_order_with_permit` → `place_order`; `modify_order_with_permit` → `modify_order`; `cancel_order_with_permit` → `cancel_order`; `convert_position_with_permit` → `convert_position`; `place_trigger_with_permit` → `place_trigger`; `modify_trigger_with_permit` → `modify_trigger`; `delete_trigger_with_permit` → `delete_trigger`. `admit(target)` has no replacement: read `Admission::headroom(class)` if you want to know capacity before calling |
| `BR-18` | Every mutation and session operation (`place_order`, `modify_order`, `cancel_order`, `convert_position`, `authorise_holdings`, `place_trigger`, `modify_trigger`, `delete_trigger`, `exchange` and `invalidate`) is a plain function returning `Dispatched<T>` instead of an `async fn`. The future's output is `Result<Response<T>, HttpError>`, not `Result<_, ManjaError>`. The call itself validates, renders the wire form, admits and spawns the single attempt on a task the client owns; dropping, timing out or selecting away from the future abandons nothing, the attempt completes and its outcome reaches the outcome stream or the orphan ring, and `HttpErrorKind::Cancelled` is never produced for a mutation by anything the caller does with the future (only the runtime shutting down with the attempt in flight still records one). A call made outside a Tokio runtime is a `Configuration` refusal (`contract.md` §2.17, `contract.md` §2.6) | `.await` the returned value as before. Before: `let receipt: Result<KiteApiResponse<OrderReceipt>, ManjaError> = client.orders().place_order(&order).await;`. After: `let receipt: Result<Response<OrderReceipt>, HttpError> = client.orders().place_order(&order).await;`, or in two steps, `let dispatched = client.orders().place_order(&order);` to read `dispatched.operation_id()` or `dispatched.refusal()` first, then `dispatched.await`. A `?` in a function returning `ManjaError` or `Box<dyn Error>` still compiles, since `HttpError` converts into `ManjaError`. Code that matched `ManjaError::Http(e)` on a mutation's error matches `e` directly, or calls `ManjaError::from`. Code that cancelled a mutation by dropping its future, or wrapped one in `tokio::time::timeout` expecting the order not to be placed, must know that the attempt now completes regardless: read the outcome from the stream (`build_with_outcomes`) or from `HttpDiagnostics::orphaned_outcomes`. Code that dispatched mutations from a thread without a Tokio runtime enters one first. A read is unchanged |
| `BR-19` | `PlaceOrderRequest::new` takes a `Tag` as its last argument and `PlaceOrderRequest::tag` is a `Tag`, not an `Option<String>`: every placement carries a tag, checked by construction (1 to 20 ASCII letters or digits), and `validate` no longer checks it. The tag is a correlation field the broker echoes, not an idempotency key; use one per attempt (`contract.md` §2.16) | Build the tag first and pass it. Before: `let mut order = PlaceOrderRequest::new(variety, exchange, "INFY", side, kind, qty, product); order.tag = Some("rebalance42".into());`. After: `let tag = Tag::new("rebalance42")?; let order = PlaceOrderRequest::new(variety, exchange, "INFY", side, kind, qty, product, tag.clone());`. Code that set no tag chooses one per attempt. Code that scanned `list_orders` for its tag calls `list_orders_tagged(&tag)` and treats more than one row as a question to answer, not a duplicate to ignore. `Order::tag` is unchanged |
| `BR-20` | The ticker decoder reads every quantity, volume, open-interest and order-count field, and both timestamps, unsigned, and never refuses a value of theirs; prices and the index change stay signed. `DECODER_VERSION` is 2. A full packet whose open interest is above `i32::MAX` (observed on a futures contract with a large lot), or a depth level with more than 32 767 orders, used to fail with `PacketErrorKind::NegativeQuantity` and an `invalid_field` diagnostic; it now decodes. `PacketErrorKind::NegativeQuantity` is removed, so `UnknownLength` is the only packet error (`contract.md` §2.9, `contract.md` §5) | Nothing to change to receive the values: the packet types were already `u32` and `u16`. Delete any match arm naming `PacketErrorKind::NegativeQuantity`, which no longer compiles, and any handling that expected an `invalid_field` diagnostic for these packets. Stored outputs carry the decoder version, so a consumer comparing outputs across versions sees 2 |
| `BR-21` | Ticker prices of BSE currency derivatives (BCD) and NSE commodity derivatives (NCO) divide by 10 000, as Zerodha's own client library does. BCD used to be refused with `ScaleError::UnsupportedSegment`, which is now deprecated and never produced; NCO is new: `Exchange::NCO` (wire `NCO`) and `Segment::Nco`. An HTTP field whose exchange is `NCO` now decodes as `Exchange::NCO` instead of an unknown value, and NCO passes order, GTT, margin and position-conversion validation as a tradable exchange. `CONVERSION_POLICY_VERSION` is 2 (`contract.md` §5) | Code that expected a BCD price to be refused now receives a scaled price. Code that matched `Inbound::Unknown` for an `NCO` exchange matches `Inbound::Known(Exchange::NCO)` instead. An exhaustive `match` on `Exchange` needs an `NCO` arm (the enum is not `#[non_exhaustive]`); a `match` on `Segment` already has a wildcard arm |

Removed with `BR-17`, one row per method, with the call that replaces it:

| Removed | Replacement |
|---|---|
| `Orders::place_order_with_permit(&request, permit)` | `Orders::place_order(&request)` |
| `Orders::modify_order_with_permit(variety, &order_id, &request, permit)` | `Orders::modify_order(variety, &order_id, &request)` |
| `Orders::cancel_order_with_permit(variety, &order_id, permit)` | `Orders::cancel_order(variety, &order_id)` |
| `Portfolio::convert_position_with_permit(&request, permit)` | `Portfolio::convert_position(&request)` |
| `Gtt::place_trigger_with_permit(&request, permit)` | `Gtt::place_trigger(&request)` |
| `Gtt::modify_trigger_with_permit(trigger_id, &request, permit)` | `Gtt::modify_trigger(trigger_id, &request)` |
| `Gtt::delete_trigger_with_permit(trigger_id, permit)` | `Gtt::delete_trigger(trigger_id)` |
| `HTTPClient::admit(target)` | none; `Admission::headroom(class)` reads capacity without reserving it |

The 0.2 public surface (`tests/public_surface.rs` at `v0.2.0`) was diffed against the
0.3.0 one. Every item that disappeared is in `BR-17` (`DispatchPermit`, `PermitTarget`,
`admit` and the seven `*_with_permit` methods), `BR-15` (`KiteApiResponse`) or `BR-20`
(`PacketErrorKind::NegativeQuantity`); every item whose shape changed is in `BR-15`
(every endpoint's return type), `BR-18` (every mutation's signature and error type),
`BR-19` (`PlaceOrderRequest::new`) or `BR-16` (the renamed `AdmissionError` variant);
the new `Exchange::NCO` variant is in `BR-21`; every item that appeared is in §4.

## 4. Additions

| Area | New API |
|---|---|
| Headroom | `Admission::headroom(class)` returning `Headroom { standard_now, control_now, next_slot_in, day_remaining }` and `Admission::modifications_remaining(&order_id)`, advisory local-view snapshots (`contract.md` §3.6) |
| Session expiry | `kite::protocol::SESSION_EXPIRY_IST` (06:00 IST) and `session_expires_at(issued_at)`, the next 06:00 IST strictly after the issue time, always available (`contract.md` §2.2) |
| Wire forms | `kite::connect::wire::WireForm` with `method`, `endpoint`, `path`, `content_type`, `body_len`, `sha256`, `sha256_hex`, `Display` and `Debug`; `PlaceOrderRequest::wire`, `ModifyOrderRequest::wire(variety, &order_id)`, `PositionConversionRequest::wire`, `HoldingsAuthorisationRequest::wire`, `GttRequest::wire` and `wire_modification(trigger_id)`; `WireForm::cancel_order`, `delete_trigger`, `session_exchange` and `session_invalidation`. A mutation sends exactly the bytes its form renders (`contract.md` §2.1) |
| Ingress stamps and source identity | `ResponseMeta::source`, `sent_at` and `received_at`; `IngressStamp { wall, monotonic }` in `kite::connect::models`; `HttpClientBuilder::identity(SourceIdentity)` and `HTTPClient::source_identity()`; `HttpError::source`, `operation_id`, `sent_at` and `received_at`; `SourceIdentity` implements `Serialize` and `Deserialize` (`contract.md` §2.18, `contract.md` §2.4) |
| Synchronous admission | `Admission::try_acquire(priority, class, order_id)`; `AdmissionError::Refused { retry_after }`, `AdmissionError::cause`, `retry_after` and `resets_at`; `HttpError::admission_cause`, `retry_after` and `resets_at`; `kite::obs::schema::AdmissionCause`; the `admission_cause` span field and the `refused` admission result (`contract.md` §2.4, `contract.md` §4.2) |
| Constructibility | `ResponseMeta::new`, `Outcome::new`, `OrderUpdate::new`, `SliceError::new`, `RequestError::new` and `PostbackChecksum::new`; `HttpError::new` and its `with_*` builders and `BrokerError::new` are public; `kite::connect::models::{SessionMeta, ProfileMeta}` name the two metadata structs that were public fields' types no caller could name; `kite::connect::models::UnknownExchange` and `kite::protocol::PostbackChecksum` are exported; `manja::secrecy` re-exports the `secrecy` crate. Every public response type can now be built outside the crate (`contract.md` §2.19) |
| Segments | `Exchange::NCO` and `Segment::Nco` for NSE commodity derivatives, whose ticker prices divide by 10 000 (`BR-21`; `contract.md` §5) |
| Tags | `kite::connect::models::Tag`, 1 to 20 ASCII letters or digits, built with `Tag::new`, `TryFrom<&str>`, `TryFrom<String>` or `FromStr`, with `as_str` and `Display`; `Orders::list_orders_tagged(&tag)`, the strict order book filtered to the rows carrying the tag (`contract.md` §2.16) |
| Dispatch | `kite::connect::dispatch`: `Dispatched<T>` with `operation_id`, `refusal`, `wire_sha256`, `correlation` and `is_in_flight`; `Outcome` and `ReceiptIds`; `OutcomeStream::recv`, `len` and `is_empty`; `OutcomeStreamError::Overflow { dropped }`; `CloseError { in_flight }`; `ORPHANED_OUTCOMES` and `MAX_CORRELATION_BYTES`; `HttpClientBuilder::build_with_outcomes`; `HTTPClient::with_correlation(token)` and `close(deadline)`; `HttpLimits::outcome_capacity` and `with_outcome_capacity` (`B-HTTP-13`); `HttpDiagnostics::dispatches_in_flight`, `orphaned_outcomes`, `orphaned_outcome_count` and `outcome_stream_dropped`; the `correlation` and `wire_sha256` fields on the `manja.http.operation` span (`contract.md` §2.17, `contract.md` §3.2, `contract.md` §4.2) |
| Priority lanes | `Priority { Standard, Control }` in `kite::connect::admission`; `HTTPClient::with_priority(priority)` returns a client whose calls take that lane; `AdmissionLimits::with_control_reserve(class, per_second)` and `control_reserve(class)` for `B-HTTP-15`; `Admission::acquire_with_priority`; `AdmissionGrant::priority`; `AdmissionError::NoStandardCapacity`; the `priority` field on the `manja.http.operation` span and its `priority` domain (`contract.md` §2.1, `contract.md` §3.2, `contract.md` §3.6, `contract.md` §4.2) |

## 5. Changes that keep their path

| Item | Change |
|---|---|
| Admission of `Standard`-class reads and other `Standard`-lane traffic | By default the `Standard` class now admits 8 `Standard`-lane requests per second, keeping 2 per second for the `Control` lane (`B-HTTP-15`). Order cancellation and GTT deletion take the `Control` lane by default and are no longer queued behind a full second of polling. A caller that needs the previous single-lane behaviour sets `AdmissionLimits::default().with_control_reserve(RateClass::Standard, 0)`; one whose own polling must not displace its urgent reads marks those calls with `with_priority(Priority::Control)`. The class total is unchanged at the documented 10 per second |

## 6. Traceability

Where each decision of the 0.3 design lands. `D12` is the list of things that stay out
of the SDK (`contract.md` §2.11) and has no code of its own.

| Decision | Contract | Verification row | This guide |
|---|---|---|---|
| D1, mutations never wait | `contract.md` §2.4 `Admission` row, `contract.md` §3.2 `B-HTTP-06`, `contract.md` §3.6, `contract.md` §5 "No wait for mutations" | `http_admission` | §2, `BR-16` |
| D2, no dispatch permits | `contract.md` §3.2 `B-HTTP-12` (retired), `contract.md` §5 "No dispatch permits" | `public_surface` (retired names absent) | `BR-17` and its removed-method table |
| D3, detached mutations and the outcome stream | `contract.md` §2.6 HTTP cancellation row, `contract.md` §2.17, `contract.md` §3.2 `B-HTTP-10`, `B-HTTP-13`, `B-HTTP-14`, `contract.md` §4.2, `contract.md` §4.5, `contract.md` §5 "Detached mutations" | `http_dispatch`, `http_session`, `http_observability`; the `http dispatch PUT` benchmark row (`verification.md` §5.3, budget owner-set) | `BR-18`, §4 "Dispatch" |
| D4, typed responses | `contract.md` §2.18, `contract.md` §5 "Responses" | `http_reads`, `http_orders`, `http_order_rows` | `BR-15` |
| D5, required tag | `contract.md` §2.16 "Tags", `contract.md` §5 "Required tag" | `http_orders` (grammar, wire, `list_orders_tagged`, compile-fail doc test) | `BR-19`, §4 "Tags" |
| D6, wire forms | `contract.md` §2.1 (from "A mutation sends exactly the bytes"), `contract.md` §5 "Wire forms" | `http_orders`, `http_gtt`, `http_session`, `http_holdings_authorisation` | §4 "Wire forms" |
| D7, priority lanes and the control reserve | `contract.md` §2.1 (from "Every operation takes a `Priority`"), `contract.md` §3.2 `B-HTTP-15`, `contract.md` §3.6, `contract.md` §4.2, `contract.md` §5 "Priority lanes" | `http_admission`, `http_observability` | §4 "Priority lanes", §5 |
| D8, source identity and ingress stamps | `contract.md` §2.4 (from "Every error an operation produces"), `contract.md` §2.18, `contract.md` §5 "Ingress stamps" | `http_reads`, `http_orders`, `envelope_capture` | §4 "Ingress stamps and source identity" |
| D9, constructible types | `contract.md` §2.19, `contract.md` §5 "Non-exhaustive response types" | `constructible` | §4 "Constructibility" |
| D10, advisory headroom | `contract.md` §3.6, `contract.md` §5 "Headroom" | `http_admission` | §4 "Headroom" |
| D11, session expiry constant | `contract.md` §2.2, `contract.md` §5 "Session expiry" | `protocol_common` (unit tests in `kite::protocol::datetime`) | §4 "Session expiry" |
| D13, synthetic fixtures | none: fixtures are not part of the contract | `fixture_manifest`, `fixture_invariants`, `decoder_packets`; `verification.md` §1.2 and `INV-GAP-08` | none: not a break, no public item changes |

## 7. Migrating from manja 0.1 to 0.2

The guide for the previous release, unchanged. Its break IDs `BR-01` to `BR-14` are
cited by the contract and stay defined here.

### 7.1 Compatibility policy

- **Paths.** Under the default features these keep their paths:
  `manja::kite::connect::client::HTTPClient`; the resources in
  `manja::kite::connect::api` (`Session`, `User`, `Orders`, `Portfolio`, `Market`,
  `Margins`, `Charges`); the model re-exports in `manja::kite::connect::models`; and
  `manja::kite::ticker::{Mode, TickerRequest}`.
- **Features.** The default is `http`, `ticker` and `decoder`. A path disappears only
  when you disable default features. With no features the crate has no Tokio or
  network dependency.
- **Minimum Rust version.** 1.88.0, with edition 2024.
- **Versioning.** Before 1.0, breaking changes ship only in minor-version bumps, each
  listed in this guide. Where practical, an item is deprecated for a release before it
  is removed. 0.2.0 is itself the breaking release, so code that only 0.1 callers
  needed is removed outright (§7.4, §7.5).
- **Safety corrections are not held back for compatibility.** Where the old behavior
  was unsafe (success reported for errors, unbounded retries, silently truncated input,
  tokens in URLs), it changed; each change is listed in §7.4.

### 7.2 Additions

| Area | New API |
|---|---|
| Credentials | `Credentials` (API key and access token), `ApiKey`, `AccessToken`, `ApiSecret`, `RequestToken`; `Debug` redacts every secret |
| Session | `HTTPClient::session(api_key)`, then `Session::exchange(&RequestToken, &ApiSecret)` and `Session::invalidate(&AccessToken)` |
| HTTP errors | `ManjaError::Http(HttpError)` with `kind()`, `stage()`, `http_status()`, `broker()`, `attempt()`, `retry()`, `is_timeout()` and `may_have_reached_broker()` |
| Admission | `Admission`, `QuotaProfile::kite_v3()` with `with_version`, `with_windows(RateClass, Vec<Window>)`, `with_daily_order_ceiling` and `with_modifications_per_order`, `AdmissionLimits`; `HttpClientBuilder::admission` shares a quota between clients |
| Scheduling | `SchedulerLimits`; `HTTPClient::admit(PermitTarget)` returns a `DispatchPermit`, used by `place_order_with_permit`, `modify_order_with_permit`, `cancel_order_with_permit`, `convert_position_with_permit`, `place_trigger_with_permit`, `modify_trigger_with_permit` and `delete_trigger_with_permit` |
| Requests | `PlaceOrderRequest`, `ModifyOrderRequest`, `PositionConversionRequest`, `OrderMarginRequest` and `OrderChargesRequest`, each with `validate()` |
| Quotes | `Quotes<Q>` with `requested`, `received`, `missing`, `unexpected`, `get` and `is_complete`; `Instrument::quote_key` |
| Accessors | `HTTPClient::portfolio()`; every resource accessor takes `&self` |
| Diagnostics | `HTTPClient::diagnostics()` returns `HttpDiagnostics` |
| Observability | `Observability` (disabled by default), `MetricRecorder`, `InMemoryRecorder`, `BridgeRecorder`; `HTTPClient::with_observability`, `HttpClientBuilder::observability`, `TickerBuilder::observability` |
| Protocol types | `InstrumentToken`, `OrderId`, `MfOrderId`, `OrderIdError`, `Quantity`, `ScaledPrice`, `Segment`, `Inbound<T>`, `OrderUpdate` |
| Envelopes | `RawObservation`, `LifecycleEvent`, `SourceKey`, `SourceIdentity`, `SourceSequencer`, `GapFacts`, `ENVELOPE_VERSION` |
| Ticker | `kite::ticker::actor`: `TickerBuilder::spawn` returns `(TickerHandle, TickerEvents, TaskGuard)`; the commands `subscribe`, `unsubscribe`, `set_mode` and `replace`; `TickerStatus`; `TickerLimits`, `ReconnectLimits`; `TickerRequest::set_mode` |
| Decoder | `kite::decoder::{framing, packets, text, adapter}`; `kite::ticker::typed::TypedEvents` with both `ticker` and `decoder` |
| Holdings authorisation | `Portfolio::authorise_holdings(&HoldingsAuthorisationRequest)` returning `HoldingsAuthorisation` with `portal_url`; `HttpError::requires_holdings_authorisation`; the `endpoint` label value `/portfolio/holdings/authorise` |
| Mutual funds | `HTTPClient::mutual_funds()` returns `MutualFunds` with `list_orders`, `get_order`, `list_sips`, `list_holdings`, `get_instruments` and `get_instruments_csv`; `MfOrder`, `MfSip`, `MfHolding`, `MfInstrument`, `MfOrderStatus`, `MfOrderVariety`, `MfPurchaseType`, `SipStatus`, `SipFrequency`, `DividendType`, `SchemeType`, `MfPlan`; the `endpoint` label values `/mf/orders`, `/mf/orders/{order_id}`, `/mf/sips`, `/mf/holdings` and `/mf/instruments` |
| Historical data | `Market::get_historical(&HistoricalRequest)` returning `HistoricalData` of `Candle`s; `CandleInterval`; `RateClass::Historical`; the `endpoint` label value `/instruments/historical/{instrument_token}/{interval}` |
| Tolerant order lists | `Orders::list_orders_with_rejections`, `get_order_history_with_rejections`, `list_trades_with_rejections` and `get_order_trades_with_rejections`, returning `Rows<T>` of `Row::Decoded(T)` or `Row::Rejected(RowError)`; the `manja_http_rejected_rows_total` counter. The strict methods are unchanged |
| GTT | `HTTPClient::gtt()` returns `Gtt` with `place_trigger`, `modify_trigger`, `delete_trigger` (each with a `_with_permit` form and a `PermitTarget::PlaceGtt`, `ModifyGtt` or `DeleteGtt`), `list_triggers` and `get_trigger`; `GttRequest` (`single`, `two_leg`, `from_trigger`, `validate()`), `GttOrderRequest`, `GttReceipt`, `GttTrigger`, `GttCondition`, `GttOrder`, `GttOrderResult`, `GttOrderOutcome`, `GttType`, `GttStatus`; the `endpoint` label values `/gtt/triggers` and `/gtt/triggers/{id}` |

### 7.3 Changes that keep their path

| Item | Change |
|---|---|
| `HTTPClient::orders`, `market`, `margins`, `charges` | Take `&self` instead of `&mut self`. Existing `&mut` call sites still compile |
| `HTTPClient::new`, `with_config` | Return `Result`: a construction failure is reported, not replaced |
| `ManjaError` | `#[non_exhaustive]` |
| `HttpError::detail`, `TextError::detail` | A response that fails to parse or decode is described by serde's error category and position, without serde's message, which quoted the offending value (`contract.md` §2.4). Code matching on the old text should use the error kind instead |

### 7.4 Intentional breaking fixes

| ID | Change | What to do |
|---|---|---|
| `BR-01` | A non-2xx status or a `status: "error"` envelope is never `Ok`, and a missing `error_type` no longer panics | Handle `Err(ManjaError::Http(e))`, using `e.kind()` and `e.stage()` |
| `BR-02` | Placement, modification and position conversion take dedicated request types and are form-encoded; `modify_order` and `cancel_order` take an `OrderVariety` | Build a `PlaceOrderRequest`, `ModifyOrderRequest` or `PositionConversionRequest`. A response-shaped `Order` is no longer accepted, because every `Order` carries response-only fields |
| `BR-03` | `get_positions` returns `Positions { net, day }` | Read `positions.net` and `positions.day` |
| `BR-04` | `Margins::orders` takes and returns arrays | Pass `&[OrderMarginRequest]` and receive `Vec<OrderMargin>` |
| `BR-05` | Quote requests over the documented limits (500 for `/quote`, 1000 for OHLC and LTP), and empty, duplicate or malformed keys, are rejected before sending, never truncated | Batch your keys, and check `quotes.missing` and `quotes.unexpected` |
| `BR-06` | Broker datetimes without an offset are `DateTime<FixedOffset>` at +05:30, nulls stay `None`, and a trade's bare time of day is an explicit variant | Update field types |
| `BR-07` | `ManjaError` and `KiteApiException` are `#[non_exhaustive]`; unknown broker error types are preserved | Add wildcard arms |
| `BR-08` | Cancellation no longer puts the API key and access token in the URL | Nothing |
| `BR-09` | Repeated 429s end within a total deadline; mutations and session operations make one attempt; the per-call `with_backoff` methods on the resources are removed | Configure `SchedulerLimits`. Retry a mutation yourself only after checking the order's state |
| `BR-10` | The browser, WebDriver and TOTP login (`kite::login`), `KiteLoginFlow`, `Session::gen_request_token`, `KiteCredentials`, the login and redirect URLs in `Config`, and the `fantoccini`, `totp-rs`, `base32` and `url` dependencies are removed | Obtain the request token yourself, call `client.session(api_key).exchange(&request_token, &api_secret)`, then `session.credentials()` |
| `BR-11` | The `ManjaError` variants `EnvVarError`, `InvalidHeaderValueError`, `WebDriverNewSessionError`, `WebDriverError`, `Reqwest`, `TotpError`, `JSONDeserialize`, `IoError` and `Internal`, and `From<&str> for ManjaError`, are removed. Every failure is now `Http` or `Credential` | Remove those match arms |
| `BR-12` | The legacy WebSocket client is removed: `WebSocketClient`, `TickerStream`, `StreamState`, `KiteStreamCredentials` and `SubscriptionStream`, and with them the `stubborn-io` dependency and the direct `tungstenite` dependency | Use `TickerBuilder` (§7.5). The ticker's WebSocket types come from `tokio_tungstenite::tungstenite` |
| `BR-13` | `TickerRequest::subscribe_with_mode` is removed. It built a `mode` request, never a subscription | For the same message, call `TickerRequest::set_mode`, which produces identical bytes. To subscribe with a mode, send `TickerRequest::subscribe` and then `set_mode`, or use the ticker handle's `subscribe` |
| `BR-14` | `kite::traits` is removed: `KiteAuth`, which the crate itself never used, and `KiteConfig`, whose `url` and `api_base` are now inherent methods of `Config` | Call `config.url(path)` and `config.api_base()` directly, and drop the trait imports. Replace a `KiteConfig` bound or `dyn KiteConfig` with `&Config`. In place of `add_auth_header`, set the `Authorization` header to `Credentials::authorization_header()`, which is `token api_key:access_token` (`kite:user.md:124`), or let `HTTPClient` send it |

Other breaking changes made with these fixes:

- `Config::from_parts(base, login, redirect, credentials)` is replaced by
  `Config::new(base)`. `Config` no longer reads environment variables, and `default()`
  is the production endpoint.
- `HTTPClient::default`, `set_user_session`, `user_session` and `http_client` are
  removed. Credentials are an immutable snapshot: `with_credentials` returns a new
  client and leaves the original unchanged.
- `HTTPClient::session` takes an `ApiKey`. `generate_session` and `delete_session` are
  replaced by `exchange` and `invalidate`, which install and clear nothing.
  `UserSession` tokens are secret-wrapped, `UserSession` is no longer `Serialize`, and
  `refresh_token` and `enctoken` are `Option`.
- Order IDs are checked types (`kite::protocol`; `contract.md` §2.16):
  - Order endpoints take `&OrderId` rather than `&str`: `modify_order`, `cancel_order`,
    their `_with_permit` forms, `get_order_history` and `get_order_trades`.
    `PermitTarget::ModifyOrder` holds an `OrderId`.
  - `MutualFunds::get_order` takes `&MfOrderId`, a separate type that also allows the
    hyphens of the documented UUIDs.
  - The `order_id` fields of `Order`, `Trade`, `OrderReceipt`, `SliceResult::Placed` and
    `OrderUpdate`, and the parent order IDs, are `OrderId`s; `GttOrderOutcome::order_id`
    is `Option<OrderId>`; `MfOrder::order_id` is an `MfOrderId`.
  - Build one with `OrderId::new(id)` or `id.parse::<OrderId>()`, which return
    `Result<OrderId, OrderIdError>`, or pass one from a response straight back.
    `OrderIdError` implements `std::error::Error`, but there is no conversion into
    `ManjaError`, so map it yourself before using `?` in a function that returns the
    crate's `Result`.
  - An invalid caller ID used to fail at call time as a `Validation` error, recorded as a
    rejected operation. It now fails when the ID is built, as an `OrderIdError`, and
    nothing is sent or recorded.
  - A response whose order ID fails the grammar is now a `Decode` error.
- `get_quotes` takes `&[&str]` keys and returns `Quotes<Q>`. `Instrument` fields are
  typed, and `to_query` is replaced by `quote_key`.
- Order, trade, holding, auction and position fields use `Inbound`, `InstrumentToken`,
  `Quantity` and `Option`, as documented on each type.
- `dotenv` is no longer a dependency, and `tracing-subscriber` is a dev-dependency
  only: install your own subscriber.

### 7.5 The removed WebSocket client

0.1's `WebSocketClient` is removed (`BR-12`). It made no readiness, reconnect or
subscription-restoration guarantee: its stream read the current socket directly, so a
lost connection ended the stream and nothing was sent again, and the requests it sent on
connecting were `mode` requests, not subscriptions, so tokens not already subscribed on
that connection received nothing. `TickerBuilder` replaces it:

| 0.1 | 0.2 |
|---|---|
| `StreamState::from_credentials(KiteStreamCredentials)` | `TickerBuilder::new(Credentials)` |
| `.subscribe_token(mode, token)` | `handle.subscribe([InstrumentToken::new(token)], mode).await`: subscribe, then mode, on every connection |
| the `KITECONNECT_WSS_API_BASE` environment variable, or the base URL in `StreamState::from_parts` | `TickerBuilder::url(...)`. The variable is no longer read, so a ticker without `url` connects to the documented production endpoint |
| `WebSocketClient::connect(state).await` | `TickerBuilder::spawn()`, which returns `(handle, events, guard)` |
| `ticker.next()` yielding a `tungstenite::Message` | `events.next()` yielding `TickerEvent::Raw(RawObservation)` or `TickerEvent::Lifecycle(LifecycleEvent)` |
| dropping the client | `handle.shutdown().await`, then drain `events` to `None`; `guard.join().await` gives the outcome |
| parsing bytes yourself | `kite::decoder` or `kite::ticker::typed::TypedEvents` |

### 7.6 Behavior you can rely on

A ticker command's `Ok(Revision)` means the owner accepted it, `CommandsSent` means it
was written to the socket, and `Active` means the desired subscriptions were written to
the connection. None of these means the broker confirmed anything or that quotes are
current. An order receipt means the broker accepted the request, not that it was filled
(`kite:orders.md:50-52`). Mutations are never retried by the SDK: a lost placement
response is reported with stage `Started`, and the order book should be checked before
placing again. Nothing the ticker has read is dropped silently; past its bounds,
delivery ends with an explicit error. Messages sent while disconnected are not
recovered, and a reconnect reports the gap instead.

`examples/ticker.rs` follows §7.5 and `examples/session.rs` follows `BR-10`. Both build and
run against local servers. `tests/public_surface.rs` compiles the old `&mut` accessor
calls and every new public path.

