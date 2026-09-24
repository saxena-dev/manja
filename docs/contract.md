# manja SDK contract

This document describes what `manja` does as a client of the Kite Connect v3 HTTP
and WebSocket APIs, and the limits within which it does it. Source code, tests and
error values refer to its sections as `docs/contract.md §N` and to its bounds by
their IDs (`B-HTTP-01`, `B-TK-05`, …). IDs are stable: a later revision may add one
but never renumbers or reuses one.

Citations of the form `kite:<page>.md:<lines>` point into the Kite Connect v3
documentation pages listed, with access time and SHA-256, in
[`kite-sources.toml`](kite-sources.toml). How each part is tested is in
[`verification.md`](verification.md), and changes from 0.1 are in
[`migration.md`](migration.md).

---

## 1. Crate layout and features

`manja` is one crate with three Cargo features:

| Feature | Modules | Adds dependencies |
|---|---|---|
| `http` | `kite::connect::{api, client, config, admission, scheduler}`, `kite::traits` | `reqwest`, `tokio` (timers and synchronization), `csv`, `hex`, `sha2` |
| `ticker` | `kite::ticker`, including `kite::ticker::actor` | `tokio` (also TCP and runtime handle), `tokio-tungstenite`, `tungstenite`, `futures-util`, `stubborn-io` (the deprecated legacy client only) |
| `decoder` | `kite::decoder` | none |

The default enables all three. `kite::connect::{credentials, models}`,
`kite::envelope`, `kite::obs`, `kite::protocol` and `kite::error` are always
available and need no async runtime. `ticker` and `decoder` together add
`kite::ticker::typed`.

Each slice depends only on the always-available modules, never on another slice:
`http` does not pull in the WebSocket stack, `ticker` pulls in neither the HTTP stack
nor the decoder, and a decoder-only or no-feature build has no Tokio, HTTP client or
WebSocket dependency. Every one of the eight feature combinations is built and tested
on the minimum supported Rust version, 1.95.0, and on 1.98.0.

---

## 2. Capabilities

### 2.1 HTTP endpoints

| Resource | Endpoints | Retry class |
|---|---|---|
| User | `GET /user/profile`, `GET /user/margins`, `GET /user/margins/{segment}` | read |
| Orders | `POST /orders/{variety}`, `PUT /orders/{variety}/{order_id}`, `DELETE /orders/{variety}/{order_id}` | mutation |
| Orders | `GET /orders`, `GET /orders/{order_id}`, `GET /trades`, `GET /orders/{order_id}/trades` | read |
| Portfolio | `GET /portfolio/holdings`, `GET /portfolio/holdings/auctions`, `GET /portfolio/positions` | read |
| Portfolio | `PUT /portfolio/positions` (conversion), `POST /portfolio/holdings/authorise` (§2.15) | mutation |
| GTT | `POST /gtt/triggers`, `PUT /gtt/triggers/{id}`, `DELETE /gtt/triggers/{id}` (§2.12) | mutation |
| GTT | `GET /gtt/triggers`, `GET /gtt/triggers/{id}` | read |
| Market | `GET /instruments`, `GET /instruments/{exchange}` (CSV), `GET /quote`, `GET /quote/ohlc`, `GET /quote/ltp` | read |
| Historical data | `GET /instruments/historical/{instrument_token}/{interval}` (§2.13) | read |
| Mutual funds | `GET /mf/orders`, `GET /mf/orders/{order_id}`, `GET /mf/sips`, `GET /mf/holdings`, `GET /mf/instruments` (CSV) (§2.14) | read |
| Margins and charges | `POST /margins/orders`, `POST /margins/basket`, `POST /charges/orders` | calculation |
| Session | `POST /session/token` (exchange), `DELETE /session/token` (invalidation) | session |

Not supported: alerts, placing or changing mutual fund orders and SIPs, the holdings
summary, the full profile, the trigger range, and receiving postbacks over HTTP.

Every JSON endpoint is decoded into its documented response type, and success requires
a 2xx status plus a `status: "success"` envelope (`kite:response-structure.md:15-28`). The
mutual fund SIP list is documented without a `status` (`kite:mutual-funds.md:219-221`),
so for that endpoint alone an absent `status` is accepted; any `status` other than
`success` is still an error.
Order placement, modification and conversion, and GTT placement and modification, take
dedicated request types and are form-encoded (`kite:response-structure.md:2`); margin and charge calculations are JSON
(`kite:margins.md:13`). Quote requests above the documented key limits (500 for
`/quote`, 1000 for OHLC and LTP, `kite:market-quotes.md:272-278`) are rejected before
sending, never truncated.

### 2.2 Session operations

`HTTPClient::session(api_key)` returns a `Session` that needs no access token.

| Operation | Contract |
|---|---|
| `exchange(&RequestToken, &ApiSecret)` | `POST /session/token` with `api_key`, `request_token` and `checksum` = SHA-256 hex of `api_key + request_token + api_secret` (`kite:user.md:97-99`). The secret is borrowed for the call and never sent, stored or logged. Returns a `UserSession` whose tokens are secret-wrapped |
| `invalidate(&AccessToken)` | `DELETE /session/token` for the session's API key (`kite:user.md:295-310`). Takes no API secret |
| Attempts | Exactly one transport attempt within the session deadline (`B-HTTP-11`). No retry after a 429, a lost response, a timeout or cancellation |
| Side effects | None beyond the request: nothing is persisted, and no client has a token installed or cleared |

### 2.3 Credentials

A `Credentials` value is an immutable snapshot of an API key and an access token. It
holds no API secret. The HTTP `Authorization: token api_key:access_token` header
(`kite:user.md:124`) and the WebSocket query (`kite:websocket.md:20`) are built
fallibly and report a configuration error rather than panicking. `Debug`, errors,
diagnostics, spans and metric labels never contain a token or a secret. Credentials are
replaced by building a new client (`HTTPClient::with_credentials`); the original client
keeps its own snapshot.

### 2.4 Errors

HTTP failures are `ManjaError::Http(HttpError)`. Every public error and error-kind enum,
like every observability label domain (§4.1), is `#[non_exhaustive]`; the stable contract is the category and the accessors, not the
`Display` text.

| `HttpErrorKind` | Meaning |
|---|---|
| `Validation` | the request failed local validation; nothing was sent |
| `Configuration` | the client or request could not be built; nothing was sent |
| `Admission` | admission capacity was unavailable before transport started |
| `Deadline` | the operation deadline expired |
| `Transport` | the transport failed or timed out |
| `HttpStatus` | a non-success status without a broker error envelope |
| `Broker` | the broker returned an error envelope (`kite:exceptions.md:18-28`) |
| `AuthRejected` | the broker rejected the credentials: `TokenException` or HTTP 403 (`kite:exceptions.md:20,35`) |
| `Decode` | a response was received but was oversized, malformed or lacked the success envelope |
| `Cancelled` | the operation was cancelled |

`HttpError` also reports the method, the endpoint template (never a URL with
credentials), the HTTP status and broker `error_type` when known, the attempt number,
retry metadata, and a **transport stage**:

| `TransportStage` | Meaning |
|---|---|
| `NotStarted` | the SDK knows the request never left the process |
| `Started` | the broker may have received the request |
| `ResponseReceived` | a response was received |
| `Unknown` | the stage cannot be established |

A timeout, a lost response or a malformed success after `Started` never implies that
the request had no effect. Unknown broker `error_type` values are preserved as strings.
The ticker's error type and the decoder's error types are separate from `ManjaError`.

### 2.5 Ticker ownership and events

`TickerBuilder::spawn` starts one owner task per ticker instance. It fails with
`TickerSpawnError::NoRuntime` outside a Tokio runtime and with
`TickerSpawnError::InvalidUrl` for a URL that is not `ws://` or `wss://` without a
query or fragment. Otherwise it returns:

| Handle | Contract |
|---|---|
| `TickerHandle` | commands and status; `Clone + Send + Sync` |
| `TickerEvents` | the one primary receiver, a `Stream<Item = Result<TickerEvent, TickerError>>`; not `Clone` |
| `TaskGuard` | the owner task's outcome, from `join()` |

The owner alone holds the socket, the desired subscriptions, the connection epochs, the
ingress sequence and the termination decision. Every binary and text message,
heartbeats included, is delivered as `TickerEvent::Raw(RawObservation)` before any
interpretation, in one source order with `TickerEvent::Lifecycle(LifecycleEvent)`.
Delivery transfers ownership to the caller; the SDK keeps no copy. With `decoder`
enabled, `kite::ticker::typed` decodes observations as they pass; it never replaces the
raw stream and opens no second socket.

### 2.6 Commands, completion, cancellation and termination

| Item | Contract |
|---|---|
| Desired state | A map from instrument token to mode. Commands: `subscribe` with an explicit mode, `unsubscribe`, `set_mode`, and `replace` of the whole map |
| Validation | Repeating an identical entry is idempotent. Conflicting modes within one replacement, setting a mode for a token that is not subscribed, and more than `B-TK-11` tokens are validation errors that change nothing |
| Completion | A command completes with `Ok(Revision)` once the owner has validated it and updated the desired state. That is acceptance, not broker acknowledgement. Lifecycle events then report the revision as `CommandsSent`, `Superseded` or `SendFailed` |
| Wire order | Subscriptions are sent before their mode changes, because a `mode` request applies to tokens already subscribed (`kite:websocket.md:30-47`). Tokens are sent in ascending order, and modes in the fixed order LTP, quote, full |
| Command cancellation | Dropping a command future after the owner accepted it does not withdraw the command |
| HTTP cancellation | Dropping an HTTP future before transport starts cancels local work; dropping it later does not cancel anything at the broker |
| End of stream | `None` only after a successful `shutdown()`. Any other end yields one `Err` first, then `None`. Reconnects are lifecycle events, never an end of stream |
| Termination | Dropping the primary receiver or the last `TickerHandle` stops the owner. `shutdown()` closes intake and the socket and delivers pending events within `B-TK-12` |

### 2.7 Ticker states and status

The connection state (`TickerState`) is one of `Disconnected`, `Connecting`,
`Restoring`, `Active`, `Backoff`, `AuthRejected`, `Stopping`, `Stopped` or `Failed`.
`Active` means the desired subscriptions were written to the current connection. It
says nothing about whether quotes are current.

`TickerHandle::status()` returns a `TickerStatus` snapshot without waiting on the data
queue: the state, the connection epoch, the desired and last-sent revisions, the age of
the last message and of the last heartbeat, the primary queue's messages, retained bytes
and oldest age, the most recent failures (`B-DIAG-01`), the terminal reason once there
is one, a snapshot revision and the snapshot time. Status changes coalesce: a reader
that falls behind sees a revision jump, and the primary stream is never delayed.

`TaskGuard::join()` resolves to a `TaskOutcome`: `Clean`, `Terminal(TerminalReason)`,
`Panicked`, or `DeadlineExpired` with the number of undelivered events. A 401 or 403
handshake response ends the ticker with `TerminalReason::AuthRejected`; it is never
retried.

### 2.8 Raw observation envelope

`RawObservation` and `LifecycleEvent` carry a `SourceKey`: producer ID, run ID, feed ID,
connection epoch and ingress sequence. Epochs are assigned per connection attempt and
never reused within a run; the ingress sequence increases within an epoch and is shared
by raw and lifecycle events. Each observation also records the receive wall time, the
monotonic time since the run started, the message kind and the unchanged payload (at
most `B-TK-10` bytes).

`ENVELOPE_VERSION` (1.0) versions this in-memory and serializable form only. The major
version changes when a field is removed, renamed or changes meaning or unit; a reader
given an unsupported major version fails with an explicit error. The minor version
changes for additive optional fields only.

A reconnect reports what is known about the missed interval (`GapFacts`). Messages sent
while disconnected are not recovered.

### 2.9 Decoder

The parser in `kite::decoder` works on borrowed bytes, never panics, has no runtime,
clock, network or global state, and is bounded by §3.4.

- **Framing** (`kite:websocket.md:63,73-85`): a binary message is either the one-byte
  heartbeat or a big-endian `u16` packet count followed by `u16`-length-prefixed
  packets. The count, every length and the absence of trailing bytes are checked
  before any packet is exposed.
- **Packets** (`kite:websocket.md:87-163`): LTP (8 bytes), quote (44), full with five
  bids and five offers (184), index quote (28) and index full (32). Each depth entry is
  quantity, price and order count followed by two padding bytes
  (`kite:websocket.md:129`); the commented byte table that follows
  (`kite:websocket.md:131-163`) contradicts that paragraph and the 184-byte total, and
  is not followed. A packet of any other length is reported as an unknown length,
  never guessed.
- **Values**: prices stay the raw `int32` on the wire. `scaled` turns one into a
  `ScaledPrice` with a caller-supplied `Segment` (§5). Negative quantities and order counts
  are refused. Timestamps stay raw Unix seconds.
- **Text** (`kite:websocket.md:165-184`): `order`, `error` and `message` messages
  become typed variants; any other `type` is preserved as `Unknown`.

`kite::decoder::adapter` attaches the source key and the pinned `DECODER_VERSION` and
`CONVERSION_POLICY_VERSION` to each decoded event. Live and captured payloads go
through the same parser and give identical results.

### 2.10 Observability handle

`Observability` is a cloneable value that defines one recording scope; clones share it.
`Observability::disabled()` is the default of every constructor. HTTP clients
(`HTTPClient::with_observability`, `HttpClientBuilder::observability`), tickers
(`TickerBuilder::observability`) and the decoder adapter accept one; the pure parser
does not. A `MetricRecorder` (`Send + Sync + 'static`) receives counters, gauges and
histograms keyed by the instruments and label sets of §4. Spans and events go through
`tracing`; the application owns the subscriber.

The SDK installs nothing globally: no subscriber, recorder, exporter, scrape endpoint or
background task. `HTTPClient::diagnostics()` and `TickerHandle::status()` work with no
recorder or subscriber.

### 2.11 What manja does not do

`manja` does not log in, obtain request tokens, store or refresh credentials, or replace
them in a running client. It does not persist or replay observations itself, coordinate
request quotas with other processes, verify postback checksums (which needs the API
secret, `kite:postbacks.md:54-56`), infer fills from order receipts, or judge whether
market data is current. It declares no Cargo feature other than `http`, `ticker` and
`decoder`.

### 2.12 GTT orders

`HTTPClient::gtt()` returns a `Gtt` resource for Good Till Triggered orders
(`kite:gtt.md`). A GTT is a trigger held by the broker: when the instrument's price
reaches a trigger value, the broker places the matching LIMIT order.

| Operation | Contract |
|---|---|
| `place_trigger(&GttRequest)` | `POST /gtt/triggers`, form-encoded: `type`, then `condition` and `orders` as JSON text (`kite:gtt.md:13-78`). Returns a `GttReceipt` with the trigger ID |
| `modify_trigger(trigger_id, &GttRequest)` | `PUT /gtt/triggers/{id}` with the complete new trigger (`kite:gtt.md:373-393`). Returns a `GttReceipt` |
| `delete_trigger(trigger_id)` | `DELETE /gtt/triggers/{id}` with no body (`kite:gtt.md:395-405`). Returns a `GttReceipt` |
| `list_triggers()` | `GET /gtt/triggers`: active triggers, and triggers in other states from the previous 7 days (`kite:gtt.md:169-172`) |
| `get_trigger(trigger_id)` | `GET /gtt/triggers/{id}`: one trigger, whatever its age or state (`kite:gtt.md:281-284`) |

`GttRequest::single` builds a trigger with one value and one order;
`GttRequest::two_leg` builds a one-cancels-other trigger with two of each
(`kite:gtt.md:80-167`). Every order is for the condition's exchange and tradingsymbol, so
the two cannot disagree. `validate()` requires a tradable exchange, a tradingsymbol of 1
to 64 bytes, exactly one trigger value and one order per leg, finite positive trigger
values and order prices, a finite non-negative last price, and LIMIT orders
(`kite:gtt.md:62`); an invalid request is a `Validation` error and nothing is sent.
`GttRequest::from_trigger` turns a fetched `GttTrigger` back into a request, the
documented way to modify one (`kite:gtt.md:390-393`). It copies the recorded last price,
which the caller updates, and fails on any value a request cannot carry: an unknown type,
exchange or order field, a zero quantity, or an order for another instrument. Trigger IDs are integers, so no
caller string reaches the path.

Placement, modification and deletion are mutations: one transport attempt, never retried
after a 429, a lost response or a timeout (`B-HTTP-04`). A `GttReceipt` acknowledges the
request only. Whether a trigger fired, and the outcome of the order it tried to place,
are in `GttTrigger::orders`, where each fired order carries a `GttOrderResult`. Trigger
types and statuses (`kite:gtt.md:359-371`) are `Inbound` values, so an undocumented one
is preserved. Every GTT endpoint is in the `Standard` quota class (§3.6).

### 2.13 Historical candles

`Market::get_historical(&HistoricalRequest)` reads the candles of one instrument and
interval: `GET /instruments/historical/{instrument_token}/{interval}` with `from`, `to`
and, when set, `continuous=1` and `oi=1` (`kite:historical.md:5-23`).

| Item | Contract |
|---|---|
| Interval | `CandleInterval`: `minute`, `3minute`, `5minute`, `10minute`, `15minute`, `30minute`, `60minute` or `day` (`kite:historical.md:14`). An undocumented interval cannot be expressed |
| Range | `from` and `to` are `DateTime<FixedOffset>` values, sent as IST wall time in the documented `yyyy-mm-dd hh:mm:ss` form (`kite:historical.md:20-21`); an instant in another zone is converted, not reinterpreted. `validate()` rejects a `to` before `from` as a `Validation` error, and nothing is sent |
| Range length | The documentation gives no maximum range per interval, so none is enforced. A response larger than the JSON body bound (`B-HTTP-08`) is a `Decode` error |
| Candles | `HistoricalData::candles`, each a `Candle` decoded from the broker's array `[timestamp, open, high, low, close, volume]` with an optional seventh value, open interest (`kite:historical.md:25-27,114-185`); a seventh value of `null` is no open interest. Timestamps keep the offset they carry. A candle with fewer than six values, more than seven, or a value of the wrong type is a `Decode` error |
| Continuous data | `continuous` asks for day candles across expired futures contracts, for NFO and MCX futures (`kite:historical.md:35-39`); the SDK only forwards the flag |
| Retries and quota | A read: retried like every read, and admitted in its own `Historical` quota class at 3 requests per second (§3.6) |

### 2.14 Mutual funds

`HTTPClient::mutual_funds()` returns a `MutualFunds` resource for funds on Zerodha's Coin
platform (`kite:mutual-funds.md`). Every operation is a read, retried like every read and
admitted in the `Standard` quota class (§3.6).

| Operation | Contract |
|---|---|
| `list_orders()` | `GET /mf/orders`: orders placed in the last 7 days (`kite:mutual-funds.md:17-19`), as `MfOrder`s |
| `get_order(order_id)` | `GET /mf/orders/{order_id}`: one order, whatever its age (`kite:mutual-funds.md:171-173`). The documented IDs are UUIDs, so an ID must be 1 to 64 ASCII letters, digits or hyphens; anything else is a `Validation` error and nothing is sent |
| `list_sips()` | `GET /mf/sips`: active and paused SIPs (`kite:mutual-funds.md:209-211`), as `MfSip`s. `instalments` and `pending_instalments` are `-1` for a SIP active until cancelled (`MfSip::is_open_ended`) |
| `list_holdings()` | `GET /mf/holdings`: allotted units (`kite:mutual-funds.md:362-364`), as `MfHolding`s. An empty `last_price_date`, as in the official sample, is `None` |
| `get_instruments()`, `get_instruments_csv()` | `GET /mf/instruments`: the CSV list of funds (`kite:mutual-funds.md:426-463`), parsed into `MfInstrument`s or returned raw, under the CSV body bound (`B-HTTP-08`). The `0` or `1` flags become booleans; any other value is a `Decode` error naming the row |

The documentation states that order placement cannot be done through the API
(`kite:mutual-funds.md:3`) and documents no endpoint that places, changes or cancels a
mutual fund order or SIP, so none is offered. Status, variety, purchase type, frequency,
dividend, scheme and plan strings are `Inbound` values whose known set is what the
documentation names in its tables or examples; the official instrument list also carries
scheme types it does not name, such as `liquid`, which are preserved as unknown.
Timestamps are IST, and dates are `yyyy-mm-dd`.

### 2.15 Holdings authorisation

Selling equity holdings needs an electronic authorisation at the depository, which the
user completes on the depository's portal by keying in their demat PIN
(`kite:portfolio.md:503-514`). A sell order that needs one fails with HTTP 428
(`kite:portfolio.md:512`), which `HttpError::requires_holdings_authorisation()` reports.

| Item | Contract |
|---|---|
| Start | `Portfolio::authorise_holdings(&HoldingsAuthorisationRequest)`: `POST /portfolio/holdings/authorise`, form-encoded as an `isin` then a `quantity` for each instrument (`kite:portfolio.md:516-524`). `HoldingsAuthorisationRequest::all()` sends no pairs, and the entire holdings are presented (`kite:portfolio.md:535`) |
| Validation | Each ISIN must be 12 ASCII uppercase letters or digits (the ISO 6166 form of the documented examples); any other ISIN is a `Validation` error and nothing is sent. Each quantity is a `Quantity`, which is positive by construction, so a zero quantity cannot be expressed |
| Result | A `HoldingsAuthorisation` with the `request_id` (`kite:portfolio.md:526-533`). It is not an authorisation: `portal_url(&ApiKey)` gives the documented URL to open in a web view or pop-up (`kite:portfolio.md:537`), with both path segments percent-encoded. When the user finishes, the portal redirects to `.../{request_id}/finish?status=success` or `status=error` (`kite:portfolio.md:539`), which the application watches for |
| Attempts | A mutation: one transport attempt, never retried after a 429, a lost response or a timeout (`B-HTTP-04`), in the `Standard` quota class (§3.6) |

The SDK opens no browser, does not watch for the redirect, and does not retry the refused
order; the application does each.

---

## 3. Runtime bounds

### 3.1 Rule for every bound

Every configurable bound is a typed field of a limits value (`HttpLimits`,
`SchedulerLimits`, `AdmissionLimits`, `TickerLimits`, `ReconnectLimits`,
`FramingLimits`, `TextLimits`) with the default, minimum and maximum below. A setter or
`build()` returns an error that names the bound's ID when a value is outside its range
or breaks a stated relation. No bound is optional, and zero never means "unlimited".
A bound marked fixed is a constant of the SDK, not a field of any limits value, and
cannot be changed.

### 3.2 HTTP

| ID | Bound | Default | Range | Relation |
|---|---|---|---|---|
| `B-HTTP-01` | Operation deadline: admission, attempts and backoff, for read, calculation and mutation operations | 30 s | 1 s to 300 s | |
| `B-HTTP-02` | Attempt timeout | 10 s | 100 ms to 120 s | ≤ `B-HTTP-01` |
| `B-HTTP-03` | Attempts for reads and calculations | 3 | 1 to 5 | |
| `B-HTTP-04` | Attempts for mutations and session operations | 1 | fixed | |
| `B-HTTP-05` | Retry backoff, initial delay / cap; multiplier 2, full jitter | 250 ms / 5 s | 10 ms to 5 s / 10 ms to 30 s | cap ≥ initial; never past `B-HTTP-01` |
| `B-HTTP-06` | Admission wait | 5 s | 1 ms to 60 s | |
| `B-HTTP-07` | Admission waiters per budget scope | 256 | 1 to 4 096 | when full, an `Admission` error |
| `B-HTTP-08` | Response body: JSON / instrument CSV | 4 MiB / 64 MiB | 64 KiB to 64 MiB / 1 MiB to 256 MiB | when exceeded, a `Decode` error with the status kept |
| `B-HTTP-09` | Request body | 1 MiB | 1 KiB to 8 MiB | when exceeded, a `Validation` error before admission |
| `B-HTTP-10` | In-flight attempts per transport | 32 | 1 to 256 | |
| `B-HTTP-11` | Session operation deadline | 15 s | 1 s to 60 s | one attempt (`B-HTTP-04`) |
| `B-HTTP-12` | Dispatch permit validity | 1 s | 10 ms to 30 s | ≤ `B-HTTP-01`; a permit is used once, and an expired one sends nothing |

Retries apply to 429, 502 to 504, transport faults and attempt timeouts, and only in
the read and calculation classes. The calculation endpoints are documented as
calculations only (`kite:margins.md:1-13,345-347`), so repeating one changes no order
or position.

### 3.3 Ticker

| ID | Bound | Default | Range | Relation |
|---|---|---|---|---|
| `B-TK-01` | Handshake timeout | 10 s | 1 s to 60 s | |
| `B-TK-02` | Connection attempts per outage / time per outage | 10 / 5 min | 1 to 100 / 10 s to 1 h | whichever is reached first ends the ticker |
| `B-TK-03` | Reconnect backoff, initial delay / cap; multiplier 2, full jitter | 500 ms / 30 s | 50 ms to 10 s / 50 ms to 300 s | cap ≥ initial; a shutdown interrupts the wait |
| `B-TK-04` | Liveness timeout: no message, heartbeat or ping | 15 s | 2 s to 120 s | a transport classification, not a statement about quotes |
| `B-TK-05` | Primary queue, messages | 4 096 | 16 to 65 536 | |
| `B-TK-06` | Primary queue retained bytes, counting whole backing buffers | 64 MiB | 1 MiB to 1 GiB | ≥ `B-TK-10` |
| `B-TK-07` | Oldest queued event age | 5 s | 100 ms to 60 s | when exceeded, delivery fails and the connection stops |
| `B-TK-08` | Primary delivery wait | 1 s | 10 ms to 30 s | ≤ `B-TK-07` |
| `B-TK-09` | Command mailbox | 64 | 1 to 1 024 | when full, a command error |
| `B-TK-10` | Payload per message, binary or text | 1 MiB | 64 KiB to 16 MiB | covers the largest full-mode message, 2 + 3 000 × (2 + 184) = 558 002 bytes |
| `B-TK-11` | Desired instruments per connection | 3 000 | 1 to 3 000 | the documented per-connection limit (`kite:websocket.md:7`) |
| `B-TK-12` | Shutdown deadline, including the close handshake | 5 s | 100 ms to 60 s | |
| `B-TK-13` | Socket writes in progress | 1 | fixed | a write interrupted by shutdown ends the ticker with `TerminalReason::SendInterrupted`; its delivery is unknown |

A consumer that falls behind ends delivery with an explicit error
(`TerminalReason::DeliveryOverload`); nothing the ticker has read is dropped silently.
A message over `B-TK-10` fails the connection; it is never truncated.

### 3.4 Decoder

| ID | Bound | Default | Range | Relation |
|---|---|---|---|---|
| `B-DEC-01` | Input payload bytes | 1 MiB | 64 KiB to 16 MiB | a live composition uses `B-TK-10` |
| `B-DEC-02` | Packets per binary message | 4 096 | 1 to 65 535 | the declared count is checked against the bound and the payload length before any packet is read |
| `B-DEC-03` | Text message bytes | 1 MiB | 1 KiB to 16 MiB | nesting depth is also limited, to 32 |

### 3.5 Diagnostics

| ID | Bound | Default | Range | Relation |
|---|---|---|---|---|
| `B-DIAG-01` | Recent failures kept per client or ticker | 16 | fixed | oldest evicted first |
| `B-DIAG-02` | Bytes kept per diagnostic text field | 512 B | fixed | redacted before it is kept; truncation is marked |
| `B-DIAG-03` | Export bridge buffer, records | 1 024 | 1 to 65 536 | when full, the record is dropped and counted in `manja_telemetry_dropped_records_total` |
| `B-DIAG-04` | Metric label value length | 64 B | fixed | every label value comes from a closed domain (§4.3) |

### 3.6 Quota profile

`Admission` is one budget scope: every client sharing it, including clones and clients
derived with `with_credentials`, draws from the same windows. `QuotaProfile::kite_v3()`
(`QUOTA_PROFILE_VERSION`) encodes the documented limits (`kite:exceptions.md:45-58`):

| Class | Endpoints | Windows |
|---|---|---|
| `Quote` | `/quote`, `/quote/ohlc`, `/quote/ltp` | 1 per second |
| `Historical` | `/instruments/historical/{instrument_token}/{interval}` | 3 per second (`kite:exceptions.md:50`) |
| `OrderPlacement` | `POST /orders/{variety}` | 10 per second, 400 per minute, 5 000 per IST day |
| `OrderModification` | `PUT /orders/{variety}/{order_id}` | 10 per second, 25 modifications per order per IST day |
| `Standard` | every other endpoint, GTT and mutual funds included | 10 per second |

An endpoint without a known class is admitted at the profile's smallest rate. A
different profile starts from `kite_v3()` and changes one setting at a time, each
validated as it is set: `with_windows(class, windows)` names the class it changes, so
windows cannot land in the wrong one; `with_daily_order_ceiling` and
`with_modifications_per_order` refuse zero; and `with_version` labels the result. Every
class needs at least one window, limits must be positive, windows at least 1 ms, and order
placement at most 10 per second. Admission cannot see requests
from other processes or other SDK instances using the same API key.

---

## 4. Observability schema

### 4.1 Version

`OBS_SCHEMA_VERSION` = 1. Instrument names, types, units, label sets, span names and
fields, and diagnostic shapes are compatibility surfaces. An addition, including a new
value in a label domain, is a documented minor change; a rename, a unit change or a
label-set change needs a new version and release notes. The catalogue is snapshotted in
`tests/obs_schema/catalogue.v1.txt`.

The Rust enums behind the label domains (`kite::obs::schema::Endpoint`, `Method`,
`QuotaClass` and the rest) and `DecodeDiagnosticKind` are `#[non_exhaustive]`, so an
added value breaks no downstream code: a `match` outside the crate needs a wildcard arm,
and each domain's `ALL` constant lists the values of the build in use.

Additions within version 1:

| Addition | Effect |
|---|---|
| `endpoint` values `/gtt/triggers` and `/gtt/triggers/{id}` (§2.12) | the domain grows from 22 to 24 values; the series bounds of the metrics labelled by `endpoint` grow with it (§4.4) |
| `endpoint` value `/instruments/historical/{instrument_token}/{interval}` (§2.13) | the domain grows to 25 values, with the §4.4 bounds |
| `endpoint` values `/mf/orders`, `/mf/orders/{order_id}`, `/mf/sips`, `/mf/holdings` and `/mf/instruments` (§2.14) | the domain grows to 30 values, with the §4.4 bounds |
| `endpoint` value `/portfolio/holdings/authorise` (§2.15) | the domain grows to 31 values, with the §4.4 bounds |
| `DecodeDiagnosticKind::InvalidField` (§4.5) | one more diagnostic kind |

### 4.2 Spans

| Span | Level | Fields |
|---|---|---|
| `manja.http.operation` | DEBUG | `operation_id`, `method`, `endpoint`, `quota_class`, `deadline_ms`, `result`, `stage` |
| `manja.http.admission` | DEBUG | `operation_id`, `quota_class`, `wait_ms`, `admission_result` |
| `manja.http.attempt` | DEBUG | `operation_id`, `attempt`, `method`, `endpoint`, `http_status`, `error_class`, `stage` |
| `manja.ticker.connection` | DEBUG | `feed_id`, `connection_epoch`, `reason`, `result` |
| `manja.ticker.restore` | DEBUG | `connection_epoch`, `desired_revision`, `instrument_count`, `sent_count`, `result` |
| `manja.ticker.command` | DEBUG | `command`, `revision`, `decision`, `rejection` |
| `manja.decode.batch` | TRACE, off by default | `source_key`, `decoder_version`, `payload_kind`, `packet_count`, `result` |
| `manja.ticker.shutdown` | DEBUG | `reason`, `elapsed_ms`, `pending_deliveries`, `result` |

HTTP attempts are children of their operation, and a caller's span context is carried
across the ticker's command mailbox. Spans and events never contain request or response
bodies, WebSocket payloads, URLs or headers with credentials, tokens, secrets or
checksums. Events fire at transitions, never once per message. The SDK adds no headers
or query parameters to Kite requests.

### 4.3 Label domains

| Key | Values |
|---|---|
| `method` | `GET`, `POST`, `PUT`, `DELETE` |
| `endpoint` | the 30 endpoint templates of §2.1, and `unknown` |
| `quota_class` | `read`, `calc`, `mut`, `sess` |
| `result` (HTTP operation) | `ok`, `http_status`, `broker_error`, `auth_rejected`, `transport_error`, `timeout`, `deadline`, `admission_rejected`, `cancelled`, `decode_error`, `validation` |
| `result` (HTTP attempt) | `ok`, `http_status`, `broker_error`, `auth_rejected`, `transport_error`, `timeout`, `cancelled`, `decode_error` |
| `error_class` | `http_429`, `http_5xx`, `transport_error`, `timeout` |
| `admission_result` | `granted`, `rejected`, `cancelled`, `deadline` |
| `transport` | `http`, `ticker` |
| `result` (ticker connection) | `ok`, `auth_rejected`, `http_status`, `transport_error`, `timeout`, `cancelled` |
| `reason` | `eof`, `remote_close`, `liveness_timeout`, `transport_error`, `protocol_error` |
| `command` | `subscribe`, `unsubscribe`, `set_mode`, `replace` |
| `decision` | `accepted`, `rejected` |
| `result` (restore) | `sent`, `send_failed`, `superseded`, `cancelled` |
| `payload_kind` | `binary`, `text` |
| `queue_role` | `command`, `raw_primary`, `pending_send`, and `notification`, which is reserved: no queue records it in this version |
| `source_mode` | `live` (observations from a running ticker), `replay` (observations the caller supplies from a capture), `standalone` (bare payloads with no source key) |
| `result` (decode) | `ok`, `partial`, `error`, `unknown_format` |
| `result` (shutdown) | `clean`, `deadline_expired`, `failed`, `panicked` |

Unknown broker strings map to `unknown` or an error value; the detail goes to bounded
diagnostics. API keys, tokens, order IDs, epochs, sequences, URLs and broker messages
are never labels.

### 4.4 Metrics

| Instrument | Type, unit | Labels | Series bound |
|---|---|---|---|
| `manja_http_operations_total` | counter, operations | `method`, `endpoint`, `quota_class`, `result` | 5 456 |
| `manja_http_operation_duration_seconds` | histogram, s | `method`, `endpoint`, `quota_class`, `result` | 5 456 |
| `manja_http_attempts_total` | counter, attempts | `method`, `endpoint`, `result` | 992 |
| `manja_http_attempt_duration_seconds` | histogram, s | `method`, `endpoint`, `result` | 992 |
| `manja_http_in_flight` | gauge, attempts | `quota_class` | 4 |
| `manja_http_admission_waiters` | gauge, waiters | `quota_class` | 4 |
| `manja_http_admission_wait_seconds` | histogram, s | `quota_class`, `admission_result` | 16 |
| `manja_http_retries_total` | counter, retries | `method`, `endpoint`, `error_class` | 496 |
| `manja_auth_rejections_total` | counter, rejections | `transport` | 2 |
| `manja_ticker_connection_attempts_total` | counter, attempts | `result` | 6 |
| `manja_ticker_connect_duration_seconds` | histogram, s | `result` | 6 |
| `manja_ticker_connections_active` | gauge, connections | none | 1 |
| `manja_ticker_reconnects_total` | counter, reconnect attempts | `reason` | 5 |
| `manja_ticker_commands_total` | counter, decisions | `command`, `decision` | 8 |
| `manja_ticker_restore_duration_seconds` | histogram, s | `result` | 4 |
| `manja_ticker_received_messages_total` | counter, messages | `payload_kind` | 2 |
| `manja_ticker_received_bytes_total` | counter, bytes | `payload_kind` | 2 |
| `manja_sdk_queue_messages` | gauge, messages | `queue_role` | 4 |
| `manja_sdk_queue_retained_bytes` | gauge, bytes | `queue_role` | 4 |
| `manja_sdk_queue_oldest_age_seconds` | gauge, s | `queue_role` | 4 |
| `manja_decode_batches_total` | counter, batches | `source_mode`, `payload_kind`, `result` | 24 |
| `manja_decode_duration_seconds` | histogram, s | `source_mode`, `payload_kind` | 6 |
| `manja_ticker_shutdown_duration_seconds` | histogram, s | `result` | 4 |
| `manja_telemetry_dropped_records_total` | counter, records | none | 1 |

A series bound is the product of its label-domain sizes. A retry is counted only when
an additional attempt starts. Received messages include heartbeats and exclude decoded
packets. A future that is never polled is not a started operation. Every histogram uses
the buckets 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60 and
120 seconds, and every duration uses monotonic time.

### 4.5 Diagnostics

| Diagnostic | Fields |
|---|---|
| `HttpDiagnostics` | active attempts, admission waiters in total and per quota class, recent failures (≤ `B-DIAG-01`) each with endpoint, method, status, error class, stage, broker error type, a redacted message (≤ `B-DIAG-02`) and a monotonic timestamp, and a snapshot revision |
| `TickerStatus` | as §2.7 |
| Decoder adapter diagnostic | source key, decoder version, packet index if any, a bounded detail (≤ `B-DIAG-02`), and a `DecodeDiagnosticKind`: `truncated`, `count_mismatch`, `trailing_bytes`, `unknown_length`, `oversized`, `invalid_text`, `unknown_text_type`, or `invalid_field` (a field value its type cannot hold, such as a negative quantity) |

Snapshots are bounded, carry revisions and timestamps so a stale view is detectable,
and need no recorder or subscriber.

### 4.6 Recorder lifetime and aggregation

- A recording scope is one `Observability` handle and all its clones. Counters are
  monotonic from the scope's creation until its last clone is dropped.
- Clones of a client share the scope and never record one operation twice.
- Within a scope, queue sizes, bytes and in-flight gauges are summed; the oldest queue
  age is the maximum, computed when collected so that it keeps advancing when progress
  stops.
- A client or ticker leaving the scope returns its gauge contributions to zero, so no
  gauge outlives its owner.

### 4.7 Rules for recorders and adapters

- Recorder and subscriber callbacks must be bounded and non-blocking and do no I/O on
  the calling path. The SDK does not defend against a callback that blocks or panics.
- The SDK's own recording is panic-free and allocates no per-message span or payload
  copy when recording is disabled.
- `BridgeRecorder` is a bounded buffer (`B-DIAG-03`) with a non-blocking enqueue. When
  full it drops and counts the record; it never retries and never reports its own
  drops through itself.
- Exporting, batching and flushing belong to the application's adapter, and cannot
  extend the ticker's shutdown deadline (`B-TK-12`).
- Values are redacted before they enter any span, event, snapshot, label or buffer.

---

## 5. Decisions

These choices are not dictated by the Kite documentation alone.

| Topic | Decision | Reason |
|---|---|---|
| Ticker price scale | Currency derivatives (CDS) divide by 10 000 000; NSE, NFO, BSE, BFO, MCX, MCXSX and indices divide by 100. The BSE currency segment (BCD) is refused | The documentation gives 10 000 000 for currencies and 100 for everything else (`kite:websocket.md:89`); nothing establishes a separate BCD scale, and the segment cannot be read from the token, so the caller supplies it |
| Handshake rejection | A 401 or 403 WebSocket handshake ends the ticker with `AuthRejected` | Retrying rejected credentials cannot succeed and would only repeat the rejection |
| Quota enforcement | Every documented window is enforced locally by default (§3.6) | Exceeding them is a documented broker error (`kite:exceptions.md:39`) |
| Oversized quote requests | Rejected before sending, never split or truncated | A split would return several snapshots taken at different times as one result |
| Endpoint support | Holdings auctions and the order charges calculation stay supported | Both are documented read or calculation endpoints with official fixtures |
| Holdings authorisation | Supported as the HTTP call that starts the flow, with the documented portal URL; the portal itself is left to the application | The endpoint and its result are documented (`kite:portfolio.md:503-541`). The remaining steps happen in the user's browser on the depository's portal, which an SDK cannot and should not drive |
| Mutual funds | Read-only: orders, SIPs, holdings and the instrument list | The documentation states that order placement cannot be done through the API (`kite:mutual-funds.md:3`) and lists only these reads (`kite:mutual-funds.md:5-11`). The official mocks still carry order and SIP placement, modification and cancellation responses, but no request for them is documented, so implementing them would mean inventing the request |
| GTT orders | Only LIMIT orders, each for the condition's instrument; a modification sends the complete trigger | The documentation lists `LIMIT` as the only order type and shows each order repeating the condition's exchange and tradingsymbol (`kite:gtt.md:56-64`); it recommends fetching the trigger and sending it back modified (`kite:gtt.md:390-393`) |
| Default features | `http`, `ticker` and `decoder` | Keeps every 0.1 import path available |
| Minimum Rust version | 1.95.0 | The oldest toolchain tested |
| Legacy ticker | `WebSocketClient` and its types are deprecated, not repaired; see `migration.md` §5 | Its stream reads the socket directly, so repairing it would change its behavior |
| Observability budgets | As in `verification.md` §5 | Measured on a recorded host; exceeding one fails the benchmark |

---

## 6. Versions

| Constant | Value | Versions |
|---|---|---|
| `ENVELOPE_VERSION` | 1.0 | the raw observation and lifecycle envelope (§2.8) |
| `DECODER_VERSION` | 1 | the decoder's output for a given input |
| `CONVERSION_POLICY_VERSION` | 1 | price scaling (§5) |
| `OBS_SCHEMA_VERSION` | 1 | the observability schema (§4) |
| `QUOTA_PROFILE_VERSION` | `kite-connect-v3/exceptions.md@2026-09-23+r2` | the default quota profile (§3.6): the page it encodes, the date that page was accessed, and a revision that increases when the encoding of the same page changes. A version without a `+r` suffix is revision 1, and the revision restarts at 1 when the page is accessed again on a new date. Revision 2 added the `Historical` class |

Each is versioned independently. The crate is pre-1.0: every intentional break ships in
a minor-version bump with release notes, and deprecated items keep working for at least
one release before removal.
