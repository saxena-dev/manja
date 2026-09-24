# Migrating from manja 0.1 to 0.2

0.2.0 is a breaking release under Cargo's rules for 0.x versions. This document lists
what was added, what changed while keeping its path, and every intentional break, with
its `BR-*` ID. The behavior you can rely on is in [`contract.md`](contract.md).

## 1. Compatibility policy

- **Paths.** Under the default features these keep their paths:
  `manja::kite::connect::client::HTTPClient`; the resources in
  `manja::kite::connect::api` (`Session`, `User`, `Orders`, `Portfolio`, `Market`,
  `Margins`, `Charges`); the model re-exports in `manja::kite::connect::models`; and
  `manja::kite::ticker::{WebSocketClient, TickerStream, StreamState,
  KiteStreamCredentials, Mode, TickerRequest}`.
- **Features.** The default is `http`, `ticker` and `decoder`. A path disappears only
  when you disable default features. With no features the crate has no Tokio or
  network dependency.
- **Minimum Rust version.** 1.95.0.
- **Deprecation.** Deprecated items keep compiling and behaving as before. They are
  removed only in a later breaking release.
- **Safety corrections are not held back for compatibility.** Where the old behavior
  was unsafe (success reported for errors, unbounded retries, silently truncated input,
  tokens in URLs), it changed; each change is listed in §4.

## 2. Additions

| Area | New API |
|---|---|
| Credentials | `Credentials` (API key and access token), `ApiKey`, `AccessToken`, `ApiSecret`, `RequestToken`; `Debug` redacts every secret |
| Session | `HTTPClient::session(api_key)`, then `Session::exchange(&RequestToken, &ApiSecret)` and `Session::invalidate(&AccessToken)` |
| HTTP errors | `ManjaError::Http(HttpError)` with `kind()`, `stage()`, `http_status()`, `broker()`, `attempt()`, `retry()`, `is_timeout()` and `may_have_reached_broker()` |
| Admission | `Admission`, `QuotaProfile::kite_v3()` with `with_version`, `with_windows(RateClass, Vec<Window>)`, `with_daily_order_ceiling` and `with_modifications_per_order`, `AdmissionLimits`; `HttpClientBuilder::admission` shares a quota between clients |
| Scheduling | `SchedulerLimits`; `HTTPClient::admit(PermitTarget)` returns a `DispatchPermit`, used by `place_order_with_permit`, `modify_order_with_permit`, `cancel_order_with_permit` and `convert_position_with_permit` |
| Requests | `PlaceOrderRequest`, `ModifyOrderRequest`, `PositionConversionRequest`, `OrderMarginRequest` and `OrderChargesRequest`, each with `validate()` |
| Quotes | `Quotes<Q>` with `requested`, `received`, `missing`, `unexpected`, `get` and `is_complete`; `Instrument::quote_key` |
| Accessors | `HTTPClient::portfolio()`; every resource accessor takes `&self` |
| Diagnostics | `HTTPClient::diagnostics()` returns `HttpDiagnostics` |
| Observability | `Observability` (disabled by default), `MetricRecorder`, `InMemoryRecorder`, `BridgeRecorder`; `HTTPClient::with_observability`, `HttpClientBuilder::observability`, `TickerBuilder::observability` |
| Protocol types | `InstrumentToken`, `Quantity`, `ScaledPrice`, `Segment`, `Inbound<T>`, `OrderUpdate` |
| Envelopes | `RawObservation`, `LifecycleEvent`, `SourceKey`, `SourceIdentity`, `SourceSequencer`, `GapFacts`, `ENVELOPE_VERSION` |
| Ticker | `kite::ticker::actor`: `TickerBuilder::spawn` returns `(TickerHandle, TickerEvents, TaskGuard)`; the commands `subscribe`, `unsubscribe`, `set_mode` and `replace`; `TickerStatus`; `TickerLimits`, `ReconnectLimits`; `TickerRequest::set_mode` |
| Decoder | `kite::decoder::{framing, packets, text, adapter}`; `kite::ticker::typed::TypedEvents` with both `ticker` and `decoder` |
| Holdings authorisation | `Portfolio::authorise_holdings(&HoldingsAuthorisationRequest)` returning `HoldingsAuthorisation` with `portal_url`; `HttpError::requires_holdings_authorisation`; the `endpoint` label value `/portfolio/holdings/authorise` |
| Mutual funds | `HTTPClient::mutual_funds()` returns `MutualFunds` with `list_orders`, `get_order`, `list_sips`, `list_holdings`, `get_instruments` and `get_instruments_csv`; `MfOrder`, `MfSip`, `MfHolding`, `MfInstrument`, `MfOrderStatus`, `MfOrderVariety`, `MfPurchaseType`, `SipStatus`, `SipFrequency`, `DividendType`, `SchemeType`, `MfPlan`; the `endpoint` label values `/mf/orders`, `/mf/orders/{order_id}`, `/mf/sips`, `/mf/holdings` and `/mf/instruments` |
| Historical data | `Market::get_historical(&HistoricalRequest)` returning `HistoricalData` of `Candle`s; `CandleInterval`; `RateClass::Historical`; the `endpoint` label value `/instruments/historical/{instrument_token}/{interval}` |
| GTT | `HTTPClient::gtt()` returns `Gtt` with `place_trigger`, `modify_trigger`, `delete_trigger`, `list_triggers` and `get_trigger`; `GttRequest` (`single`, `two_leg`, `from_trigger`, `validate()`), `GttOrderRequest`, `GttReceipt`, `GttTrigger`, `GttCondition`, `GttOrder`, `GttOrderResult`, `GttOrderOutcome`, `GttType`, `GttStatus`; the `endpoint` label values `/gtt/triggers` and `/gtt/triggers/{id}` |

## 3. Changes that keep their path

| Item | Change |
|---|---|
| `HTTPClient::orders`, `market`, `margins`, `charges` | Take `&self` instead of `&mut self`. Existing `&mut` call sites still compile |
| `HTTPClient::new`, `with_config` | Return `Result`: a construction failure is reported, not replaced |
| `ManjaError` | `#[non_exhaustive]` |
| `TickerRequest::subscribe_with_mode` | Deprecated. It always built a `mode` request, never a subscription; `TickerRequest::set_mode` gives the same output under the right name |
| `WebSocketClient`, `TickerStream`, `StreamState`, `KiteStreamCredentials`, `SubscriptionStream` | Deprecated with corrected documentation (§5). Behavior and the item type `Result<tungstenite::Message, tungstenite::Error>` are unchanged |

## 4. Intentional breaking fixes

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
| `BR-11` | The `EnvVarError`, `InvalidHeaderValueError`, `WebDriverNewSessionError`, `WebDriverError`, `Reqwest` and `TotpError` variants are removed | Remove those match arms |

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
- `get_quotes` takes `&[&str]` keys and returns `Quotes<Q>`. `Instrument` fields are
  typed, and `to_query` is replaced by `quote_key`.
- Order, trade, holding, auction and position fields use `Inbound`, `InstrumentToken`,
  `Quantity` and `Option`, as documented on each type.
- `kite::traits` requires the `http` feature and keeps only `KiteConfig::{url,
  api_base}` and `KiteAuth`.
- `dotenv` is no longer a dependency, and `tracing-subscriber` is a dev-dependency
  only: install your own subscriber.

## 5. The legacy WebSocket client

`WebSocketClient` is deprecated, not repaired. It makes no readiness, reconnect or
subscription-restoration guarantee. Its stream reads the current socket directly, so a
lost connection ends or errors the stream and nothing is sent again. The requests it
sends on connecting are `mode` requests, not subscriptions, so tokens not already
subscribed on that connection receive nothing. `StreamState::from_credentials` reads
`KITECONNECT_WSS_API_BASE` from the environment when it is set. Repairing the client
would change its observable behavior, so the corrected behavior is in the new ticker
instead.

| Legacy | Replacement |
|---|---|
| `StreamState::from_credentials(KiteStreamCredentials)` | `TickerBuilder::new(Credentials)` |
| `.subscribe_token(mode, token)` | `handle.subscribe([InstrumentToken::new(token)], mode).await`: subscribe, then mode, on every connection |
| `WebSocketClient::connect(state).await` | `TickerBuilder::spawn()`, which returns `(handle, events, guard)` |
| `ticker.next()` yielding a `tungstenite::Message` | `events.next()` yielding `TickerEvent::Raw(RawObservation)` or `TickerEvent::Lifecycle(LifecycleEvent)` |
| dropping the client | `handle.shutdown().await`, then drain `events` to `None`; `guard.join().await` gives the outcome |
| parsing bytes yourself | `kite::decoder` or `kite::ticker::typed::TypedEvents` |

## 6. Behavior you can rely on

A ticker command's `Ok(Revision)` means the owner accepted it, `CommandsSent` means it
was written to the socket, and `Active` means the desired subscriptions were written to
the connection. None of these means the broker confirmed anything or that quotes are
current. An order receipt means the broker accepted the request, not that it was filled
(`kite:orders.md:50-52`). Mutations are never retried by the SDK: a lost placement
response is reported with stage `Started`, and the order book should be checked before
placing again. Nothing the ticker has read is dropped silently; past its bounds,
delivery ends with an explicit error. Messages sent while disconnected are not
recovered, and a reconnect reports the gap instead.

`examples/ticker.rs` follows §5 and `examples/session.rs` follows `BR-10`. Both build and
run against local servers. `tests/public_surface.rs` compiles the old `&mut` accessor
calls and every new public path.
