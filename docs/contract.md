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
| `http` | `kite::connect::{api, client, config, admission, scheduler, wire, dispatch}` | `reqwest`, `tokio` (timers, synchronization and the runtime handle a dispatched mutation's attempt is spawned on, §2.17), `csv`, `hex`, `sha2` |
| `ticker` | `kite::ticker`, including `kite::ticker::actor` | `tokio` (also TCP and runtime handle), `tokio-tungstenite`, `futures-util` |
| `decoder` | `kite::decoder` | none |

The default enables all three. `kite::connect::{credentials, models}`,
`kite::envelope`, `kite::obs`, `kite::protocol` and `kite::error` are always
available and need no async runtime. `ticker` and `decoder` together add
`kite::ticker::typed`.

Each slice depends only on the always-available modules, never on another slice:
`http` does not pull in the WebSocket stack, `ticker` pulls in neither the HTTP stack
nor the decoder, and a decoder-only or no-feature build has no Tokio, HTTP client or
WebSocket dependency. Eight feature configurations are built and tested on the minimum
supported Rust version, 1.88.0, and on the stable channel: no features, each feature
alone, `ticker` with `decoder`, `http` with `ticker`, all three, and the default.

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

Every JSON endpoint is decoded into its documented response type and returned as a
`Response<T>` with the facts of the call (§2.18), and success requires
a 2xx status plus a `status: "success"` envelope (`kite:response-structure.md:15-28`). The
mutual fund SIP list is documented without a `status` (`kite:mutual-funds.md:219-221`),
so for that endpoint alone an absent `status` is accepted; any `status` other than
`success` is still an error.
Order IDs used in request paths, and those in responses, are checked types, valid by
construction (§2.16).
Every operation takes a `Priority`, `Standard` or `Control`, which chooses its admission
lane (§3.6). The caller marks it per call by making the call through
`HTTPClient::with_priority(priority)`, a client sharing the original's transport,
admission scope and credentials. Without a mark, `DELETE /orders/{variety}/{order_id}`
and `DELETE /gtt/triggers/{id}` are `Control` and every other operation is `Standard`;
a mark replaces the default in either direction. The SDK never decides which traffic
is control: it honours the mark and keeps the reserve (`B-HTTP-15`).
Order placement, modification and conversion, and GTT placement and modification, take
dedicated request types and are form-encoded (`kite:response-structure.md:2`); margin and charge calculations are JSON
(`kite:margins.md:13`).
A mutation sends exactly the bytes its `wire()` returns. Every mutation has a wire form,
`kite::connect::wire::WireForm`: the HTTP method, the endpoint template, the request
path with its segments filled in, the content type and the exact form-encoded body.
`PlaceOrderRequest::wire()`, `ModifyOrderRequest::wire(variety, &order_id)`,
`PositionConversionRequest::wire()`, `HoldingsAuthorisationRequest::wire()`,
`GttRequest::wire()` (placement) and `GttRequest::wire_modification(trigger_id)` validate
the request first and return the form or the `Validation` error the call would return;
the mutations without a request type come from `WireForm::cancel_order(variety,
&order_id)`, `WireForm::delete_trigger(trigger_id)`, `WireForm::session_exchange(...)` and
`WireForm::session_invalidation(...)`. The call builds the same form and copies its path,
query, content type and body into the request, so `sha256(body as received by Kite) ==
wire.sha256()` for every mutation; a mutation with no body (cancellation, GTT deletion,
session invalidation, an entire-holdings authorisation) sends no `Content-Type` and
hashes the empty body. `Display` renders the body as Kite receives it, for example
`tradingsymbol=INFY&exchange=NSE&...`; `sha256()` is the raw digest and `sha256_hex()`
its hex. A `WireForm` holds no credential type and no header: the `Authorization` header
is attached at send time and is never part of it, and no accessor returns the body bytes;
they leave the form only through `Display`, the hash and the send. The two session forms
carry the documented `request_token` and `checksum` fields or `access_token` query
(§2.2): the exchange form's `Display` and `Debug` print `<redacted>` for `request_token`
and `checksum`; the invalidation form has no body, so its `Display` is empty, and its
`Debug` prints the query with `access_token` as `<redacted>`. Quote requests above the documented key limits (500 for
`/quote`, 1000 for OHLC and LTP, `kite:market-quotes.md:272-278`) are rejected before
sending, never truncated.

### 2.2 Session operations

`HTTPClient::session(api_key)` returns a `Session` that needs no access token.

| Operation | Contract |
|---|---|
| `exchange(&RequestToken, &ApiSecret)` | `POST /session/token` with `api_key`, `request_token` and `checksum` = SHA-256 hex of `api_key + request_token + api_secret` (`kite:user.md:97-99`). The secret is borrowed for the call and never sent, stored or logged. Returns a `Response<UserSession>` whose tokens are secret-wrapped. The request is `WireForm::session_exchange(&api_key, &request_token, &api_secret)`, whose `Display` and `Debug` redact the token and the checksum (§2.1) |
| `invalidate(&AccessToken)` | `DELETE /session/token` for the session's API key (`kite:user.md:295-310`). Takes no API secret and returns the broker's `Response<bool>`. The request is `WireForm::session_invalidation(&api_key, &access_token)`: no body (so an empty `Display`), the two query parameters, the access token redacted in `Debug` |
| Attempts | Exactly one transport attempt within the session deadline (`B-HTTP-11`). No retry after a 429, a lost response, a timeout or cancellation |
| Side effects | None beyond the request: nothing is persisted, and no client has a token installed or cleared |
| Expiry | An access token stays valid until it is invalidated or until the next 6 AM IST after it was issued, which Kite words as "6 AM on the next day" (`kite:user.md:115`): a token issued at 03:00 IST expires at 06:00 that same morning, one issued at 06:00:00 IST at 06:00 the next day. The rule is published, with no feature gate, as `kite::protocol::SESSION_EXPIRY_IST` (06:00:00, a `NaiveTime`) and the pure function `session_expires_at(issued_at)`, which returns the next 06:00 IST strictly after `issued_at` at offset +05:30, whatever offset the input carries. Neither reads a clock, starts a timer or refreshes anything; nothing in the SDK acts on the time |

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
| `Admission` | admission capacity was unavailable before transport started. A mutation or session operation never waits for it: with a window full it is refused at once and the error carries `retry_after()`, the time until the lane's next slot; with the daily ceiling spent it carries `resets_at()`, the next 00:00 IST; the 25-per-order modification limit (`kite:exceptions.md:58`) has no recovery that day. `admission_cause()` names the cause (`window_full`, `daily_ceiling`, `modification_ceiling`, `no_standard_capacity`, `waiters_full`, `wait_expired`), for a read's exhausted wait too, including the `Deadline` error a read gets when the operation deadline expires during its admission wait. The SDK never waits a hint out; the caller decides. A mutation dispatched after `close` began, or when every `B-HTTP-10` in-flight slot is taken, is refused with this kind too, before admission, with no cause and no hint: nothing was sent, and the detail names the reason (§2.17) |
| `Deadline` | the operation deadline expired |
| `Transport` | the transport failed or timed out |
| `HttpStatus` | a non-success status without a broker error envelope |
| `Broker` | the broker returned an error envelope (`kite:exceptions.md:18-28`) |
| `AuthRejected` | the broker rejected the credentials: `TokenException` or HTTP 403 (`kite:exceptions.md:20,35`) |
| `Decode` | a response was received but was oversized, malformed or lacked the success envelope |
| `Cancelled` | a read or calculation whose future was dropped; recorded in `HTTPClient::diagnostics`, never returned by a call. Not produced for a mutation or session operation by anything the caller does with the future: their attempt runs to completion (§2.17). The one exception is the runtime itself shutting down with the attempt in flight: the attempt task is dropped with it, a `Cancelled` entry with the attempt's stage is recorded, and the `Dispatched` future, if awaited, resolves with a `Transport` error at stage `Started` saying the attempt task ended without delivering an outcome |

Every error an operation produces also carries the client's `source()` identity and the
operation's `operation_id()` (§2.18), whatever its kind and stage. `sent_at()` is `Some`
once the attempt was dispatched, that is for stage `Started` or later, and `received_at()`
is `Some` only once a response head arrived, stage `ResponseReceived`; an error with stage
`NotStarted` has neither stamp. The stamps are the same `IngressStamp`s a response
carries, from the same clocks.

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

An error's **detail** is bounded, sanitized free text for people, not a stable format. It
never holds a value taken from a response body. A body that fails to parse or decode is
described by what failed, serde's error category (`syntax error`, `data error` or
`unexpected end of input`) and, when known, its line and column, such as `malformed JSON
success body: syntax error at line 1 column 28` or `the payload does not match the
endpoint's type: data error`. A malformed CSV row is named by its row number, and a strict
list rejection by its count and first index (§2.16). serde's own message is left out,
because it quotes the offending value. The broker's `message` in an error envelope is kept,
bounded, as the broker error's message. It is the broker's own text, not a decoded value,
and may name amounts or IDs, so `Display` of a broker error can carry them.

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
| HTTP cancellation | Dropping a read or calculation future before transport starts cancels local work; dropping it later does not cancel anything at the broker. A mutation or session operation cannot be cancelled by dropping its future: the call dispatched the attempt to a task the client owns, which finishes and reports whatever the caller does (§2.17) |
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
  become typed variants; any other `type` is preserved as `Unknown`. A `TextError`'s
  detail follows the rule for HTTP error details (§2.4): what failed and where, never a
  value from the message.

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

Each mutation is admitted on its own call, now or refused with a hint (§3.6); no
per-trigger limit is documented, so GTT mutations share the `Standard` class.

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
| `get_order(&MfOrderId)` | `GET /mf/orders/{order_id}`: one order, whatever its age (`kite:mutual-funds.md:171-173`). The ID is an `MfOrderId` (§2.16), because the documented IDs are UUIDs |
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

### 2.16 Order IDs

An order ID becomes a path segment of the order endpoints, so every one is a checked
type in `kite::protocol`, and a value that could change the path cannot be constructed.

| Type | Grammar | Used by |
|---|---|---|
| `OrderId` | 1 to 64 ASCII letters or digits | `Orders::modify_order`, `cancel_order`, `get_order_history` and `get_order_trades`; the `order_id` of `Order`, `Trade`, `OrderReceipt`, `SliceResult::Placed` and `OrderUpdate`, `Order::parent_order_id` and `OrderUpdate::parent_order_id`, and `GttOrderOutcome::order_id` |
| `MfOrderId` | 1 to 64 ASCII letters, digits or hyphens | `MutualFunds::get_order` and `MfOrder::order_id` |

Both are built with `new`, `TryFrom<&str>`, `TryFrom<String>` or `FromStr`, and deserialize
through the same check. A failure is an `OrderIdError` naming the field. The two types are
distinct, so a mutual fund ID cannot be passed where an equity ID is expected, or the other
way round. An ID read from a response goes straight back into an order endpoint with no
conversion.

Every equity order ID in the official samples is a string of digits, and every mutual fund
order ID a UUID. A response whose ID fails its grammar is a `Decode` error rather than an
unvalidated ID, so no broker value can reach a request path unchecked. In the default list
methods the error covers the whole response: one malformed row, whatever the field, fails
an entire order book or trade list. For a ticker order update, it fails only that text
message.

Each equity order and trade list therefore has a second, tolerant method:
`list_orders_with_rejections`, `get_order_history_with_rejections`,
`list_trades_with_rejections` and `get_order_trades_with_rejections`. Each returns
`Rows<T>`: every element in the broker's array order, as `Row::Decoded(T)` or
`Row::Rejected(RowError)`, where the `RowError` carries the element's `index`, the
decoding `error` and the `raw` element. `Rows::items`, `rejected`, `is_complete` and
`into_complete` read it. Such a response may be partial, and every row can be rejected,
which leaves `items` empty although the broker sent rows; `is_complete` or `into_complete`
tells that apart from an empty list. `RowError`'s `Debug` prints only the index, because
the error text and the raw element can quote broker values.

Both methods decode through one path, with the policy applied inside the attempt's
classification. The envelope is classified as for every JSON response, and `data` must be
an array; its elements are then decoded one by one, so an array of non-objects is a list
of rejected rows rather than a type mismatch. A strict rejection is a `Decode` error whose
detail gives only the rejected count and the first index, such as `3 of 10 rows rejected;
first at index 7`. It is recorded as a
failed operation and attempt, in the span and in `HttpDiagnostics`. Under both policies
the rejected rows of a response are counted once in `manja_http_rejected_rows_total`, and a
complete response records nothing there. Single-object responses, and the mutual fund,
holdings and GTT lists, stay strict. The order ID in an
`OrderChargesRequest` is not an `OrderId`: the documentation allows any string there
(`kite:margins.md:395`), and it travels in the JSON body, never in a path.

### 2.17 Dispatch

Every mutation and session operation (the `mutation` and `session` rows of §2.1) is a
plain function returning `Dispatched<T>` (`kite::connect::dispatch`), a future whose
output is `Result<Response<T>, HttpError>`. The call itself, before the future is polled,
validates the request and renders its wire form (§2.1), checks the body bound
(`B-HTTP-09`), takes an in-flight slot (`B-HTTP-10`), admits the operation now or
refuses it (§2.4, §3.6) and, once admitted, hands the single attempt to a task the client
owns on the current Tokio runtime. From that point the attempt runs to completion whatever
the caller does with the future: dropping it, timing it out or selecting away from it
abandons nothing, and `HttpErrorKind::Cancelled` is never produced for a mutation by the
caller's handling of the future. Only the runtime shutting down with the attempt in flight
drops the attempt task, which is recorded as `Cancelled` with its stage (§2.4). Reads
and calculations are unchanged: `async fn`, retried within their deadline, cancelled by
drop (§2.6).

| Item | Contract |
|---|---|
| Before polling | `operation_id()`; `refusal()`, the error when the call was refused before transport, else `None`; `wire_sha256()`, the hash of the form that was sent or would have been (`None` only when the request failed validation and has no form); `correlation()`; `is_in_flight()`. `Dispatched` is `Send`, so it can be moved to another task or held in a `select!` |
| Refusal | A call refused before transport (a `Validation`, `Configuration` or `Admission` error, stage `NotStarted`) spawns nothing, emits no outcome record and resolves at once with the refusal when awaited; `refusal()` exposes the same error before the await. A dispatch made outside a Tokio runtime is a `Configuration` refusal; one made after `close` began is an `Admission` refusal; one that finds every `B-HTTP-10` slot taken is an `Admission` refusal, never a wait |
| Outcome | The result of an admitted attempt is delivered once to the `Dispatched` future if it is still alive, and always, as an `Outcome` record, to the client's outcome stream when one was registered at build time (`HttpClientBuilder::build_with_outcomes`, capacity `B-HTTP-13`). Without a stream, the outcome of a dropped future goes to the orphaned outcome ring in `HttpDiagnostics` (`B-HTTP-14`) and `orphaned_outcome_count` grows by one, whether the future was dropped before the attempt completed or afterwards with the outcome delivered to it but never polled out; only an outcome the caller took by awaiting the future goes nowhere else |
| Outcome record | `operation_id`, `correlation`, `method`, `endpoint` (the template), `wire_sha256`, `stage`, `result` (the `result.http_operation` domain, §4.3), `error` (the kind, when the attempt failed), `http_status`, `attempt` (always 1), `receipt` (the `order_id` or `trigger_id` the receipt carried, when the response decoded; otherwise none), `source`, `sent_at` and `received_at` (§2.18). It carries no request or response body, no URL and no header |
| Stream | `OutcomeStream::recv()` yields `Some(Ok(outcome))` in completion order; `len()` and `is_empty()` read what is buffered. When the consumer falls behind by `B-HTTP-13` outcomes, or drops the stream, the client keeps every attempt running, counts each outcome it could not deliver in `HttpDiagnostics::outcome_stream_dropped` and keeps it in the ring (`B-HTTP-14`); once the buffered outcomes are drained, `recv()` yields one `Some(Err(OutcomeStreamError::Overflow { dropped }))` and then `None`. Nothing is dropped silently |
| Correlation | `HTTPClient::with_correlation(token)` returns a client sharing the original's transport, admission scope, credentials and lane whose every call records `token` in the `correlation` field of its operation span (§4.2), and whose every mutation carries it in the outcome record and on `Dispatched::correlation()`; it is never a metric label. A token is 1 to `MAX_CORRELATION_BYTES` (64) bytes of printable ASCII, space included; anything else is a `Validation` error from `with_correlation` itself, returned as a plain value: no call was made, so no operation span, metric or diagnostics entry records it. A client without a mark carries none |
| Wire hash on the span | The operation span records `wire_sha256` (hex) as soon as the form is rendered, so a dispatch refused after that point is recorded with the hash of the form it did not send (§4.2) |
| Close | `HTTPClient::close(deadline)` marks the shared transport closing, so every later mutation dispatch from any clone is refused, and waits for the attempts in flight to complete and deliver; past the deadline it returns `CloseError { in_flight }` naming how many still run, and those continue on the runtime and still deliver. Dropping the last clone without `close` lets the attempts in flight finish and deliver to the stream |
| Diagnostics | `HttpDiagnostics` reports `dispatches_in_flight`, the ring `orphaned_outcomes` (≤ `B-HTTP-14`, oldest evicted first), `orphaned_outcome_count` and `outcome_stream_dropped` (§4.5) |

### 2.18 Responses

Every JSON endpoint returns `Response<T> { data: T, meta: ResponseMeta }`
(`kite::connect::models`). The CSV instrument lists return `String` or `Vec<Instrument>`
and `Vec<MfInstrument>` as §2.1 and §2.14 say.

| Field | Contract |
|---|---|
| `data: T` | The endpoint's documented type, the value itself. A call returns `Ok` only when Kite answered with a 2xx status, a `status: "success"` envelope and data of the expected shape (§2.1), so there is no `Option` and nothing to unwrap; every failure, including an error envelope, is a `ManjaError` (§2.4) |
| `meta.message: Option<String>` | Kite's optional `message` from the envelope, as received; `None` when the envelope has none |
| `meta.http_status: u16` | The HTTP status of the attempt that produced the response |
| `meta.operation_id: u64` | The operation's ID: the `operation_id` field of its `manja.http.operation` span (§4.2), unique within the client's observability scope |
| `meta.attempt: u32` | The number of the attempt that produced the response: 1 for a first try, 2 after one retry; the same numbering as `HttpError::attempt` |
| `meta.source: SourceIdentity` | The client's producer, run and feed: the type and frame the ticker's envelope carries (§2.8), so HTTP evidence and ticker evidence from one process cite the same run. Set with `HttpClientBuilder::identity`; without it the client generates one (`SourceIdentity::generate()`: producer `manja`, a fresh run ID, feed `default`). Every clone and derived handle of a client shares it; `HTTPClient::source_identity()` reads it |
| `meta.sent_at: IngressStamp` | When the attempt that produced the response was dispatched: the last instant before its bytes left the process |
| `meta.received_at: IngressStamp` | When the response head (status and headers) arrived; never earlier than `sent_at` on the monotonic clock. The wall parts are two readings of the system clock, which may step between them (§2.8) |

An `IngressStamp` is one instant in both of the envelope's clocks: `wall`, a `ReceiveTime`
(signed UTC Unix nanoseconds from the system clock), and `monotonic`, a `MonotonicElapsed`
(nanoseconds since the client's run origin, the instant its transport was built). Stamps
from one client and its clones are comparable with each other; stamps from different
clients are not, as with the ticker. Errors carry the same facts (§2.4).

`Response::map(f)` applies `f` to the data and keeps the meta. `Response<T>` has exactly
these two public fields; `ResponseMeta` is `#[non_exhaustive]` with public fields. Both
serialize; `SourceIdentity` serializes as its three identifiers, as a `SourceKey` does,
and an identifier that `SourceIdentity::new` refuses is refused on the way back in. The 0.2 envelope `KiteApiResponse`, with its
`status`, `Option<T>` data and `error_type`, no longer exists: `status` was always
`"success"` and `error_type` always `None` on a returned value, so neither was data
(`migration.md` §7).

Broker state arrives unchanged. An order row keeps every field reservation logic depends
on exactly as the broker reports it: `status`, `filled_quantity`, `pending_quantity` and
`cancelled_quantity`, with a status outside the documented tables preserved as
`Inbound::Unknown` carrying its wire text, never mapped to a documented one. The SDK does
not tell a partial fill of an open order from a terminal partially filled order; it
reports the fields that let the caller do so.

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
| `B-HTTP-06` | Admission wait, for the read and calculation classes only | 5 s | 1 ms to 60 s | A mutation or session operation never waits: it is admitted now or refused with a hint (§2.4, §3.6) |
| `B-HTTP-07` | Admission waiters per budget scope | 256 | 1 to 4 096 | when full, an `Admission` error |
| `B-HTTP-08` | Response body: JSON / instrument CSV | 4 MiB / 64 MiB | 64 KiB to 64 MiB / 1 MiB to 256 MiB | when exceeded, a `Decode` error with the status kept |
| `B-HTTP-09` | Request body | 1 MiB | 1 KiB to 8 MiB | when exceeded, a `Validation` error before admission |
| `B-HTTP-10` | In-flight attempts per transport | 32 | 1 to 256 | counts a mutation's detached attempt from the call that dispatched it; a mutation that finds no slot is refused with an `Admission` error, never queued, while a read waits for one (§2.17) |
| `B-HTTP-11` | Session operation deadline | 15 s | 1 s to 60 s | one attempt (`B-HTTP-04`) |
| `B-HTTP-12` | Retired in 0.3.0 with the dispatch permit (it bounded a permit's validity); the ID is never reused | | | |
| `B-HTTP-13` | Outcome stream capacity: outcome records buffered for the consumer registered at build time | 1 024 | 16 to 65 536 | when full, the stream ends with `Overflow` once drained; every attempt still completes and every undelivered outcome is counted and kept in the ring (§2.17) |
| `B-HTTP-14` | Orphaned outcome ring: outcome records kept in `HttpDiagnostics` for mutations whose future was dropped with no stream registered, and for outcomes the stream could not take | 16 | fixed | oldest evicted first; `orphaned_outcome_count` and `outcome_stream_dropped` count every record whether or not it is still in the ring (§2.17) |
| `B-HTTP-15` | Control reserve per quota class, in requests per second: the part of the class rate only `Control` traffic may use | 2 for `Standard`, 0 for `Quote`, `Historical`, `OrderPlacement`, `OrderModification` and `Unclassified` | 0 to 5 | For a window of period `p` it reserves `reserve × p` slots, rounded up; ceilings have no reserve. A reserve at or above a window's rate leaves the class no `Standard` capacity: every `Standard` request in it is refused at once with an `Admission` error naming this bound, and never waits. 0 makes the lanes equal |

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

Within a class, every request is admitted on one of two lanes (§2.1). `Control` traffic
may fill each window of the class entirely. `Standard` traffic may fill a window only up
to its limit minus the class's control reserve (`B-HTTP-15`, §3.2), so with the default
profile and reserve the `Standard` class admits 8 `Standard` requests per second and 10
in all, and a `Control` request is admitted while the reserved slots are free whatever
`Standard` traffic has done. Both lanes draw from the same windows and ceilings: a
`Control` request counts against the class rate, the daily order ceiling and the
per-order modification limit like any other, so the lanes together never exceed the
documented rate. A read or calculation whose lane share is full waits for its slot under
`B-HTTP-06`. A mutation or session operation is admitted synchronously: if every window of
its class has a free slot on its lane it starts at once, and otherwise the call is refused
before transport with an `Admission` error carrying the exact time until the lane's next
slot, the instant the daily ceiling resets, or the per-order modification ceiling (§2.4).
There is no wait and no option to wait: a refusal is affirmative evidence that nothing
was sent, and the hint is accurate to the instant the wait would have ended. The reserve is a local view: it cannot keep capacity from other processes
or the Kite app.

Capacity is readable. `Admission::headroom(class)` returns a `Headroom` snapshot:
`standard_now` and `control_now`, the admissions each lane would be granted at that
instant (each window's lane share minus its live admissions, the smallest window
deciding; both 0 once the daily ceiling is spent); `next_slot_in`, the time until the
`Standard` lane next has a slot when it has none, and `None` when it has one now, when
the ceiling is spent (no window frees it) or when the control reserve covers a window's
rate (the lane can never be admitted); and `day_remaining`, the placements left under
the daily ceiling for `OrderPlacement` and `None` for every other class.
`Admission::modifications_remaining(&order_id)` is the modifications that order may
still make today under the per-order limit. Both are **advisory** and the **local
view**: reading reserves nothing, capacity can be taken between the read and the call,
and the scope cannot see other processes, other SDK instances or the Kite app, whose
orders count against the same limits (`kite:exceptions.md:45-58`; the daily ceiling is
per user and API key, `kite:exceptions.md:57`). Within one scope the snapshot agrees
with the next admission decision on the windows and the daily ceiling: if a lane reports
`n`, `n` calls on it admit at once and the next waits or is refused, subject for
`OrderModification` to the order's own limit, which `modifications_remaining` reads. There
is no reserving variant.

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
| `manja_http_rejected_rows_total` (§2.16) | a new counter of list rows that did not decode, labelled by `endpoint` only |
| `DecodeDiagnosticKind::InvalidField` (§4.5) | one more diagnostic kind |
| `priority` field on `manja.http.operation` and the `priority` domain `standard`, `control` (§2.1, §4.2, §4.3) | a span field and a domain; no metric gains a label, so no series bound changes |
| `admission_cause` field on `manja.http.operation`, the `admission_cause` domain, and the `admission_result` value `refused` (§2.4, §4.2, §4.3) | a span field, a domain and one more value of an existing label domain: the `manja_http_admission_wait_seconds` series bound grows from 16 to 20 (§4.4) |
| `correlation` and `wire_sha256` fields on `manja.http.operation` (§2.17, §4.2) | two span fields: `correlation` is set on every call made through a `with_correlation` client and `wire_sha256` on every mutation with a wire form; neither is a metric label, so no series bound changes |

### 4.2 Spans

| Span | Level | Fields |
|---|---|---|
| `manja.http.operation` | DEBUG | `operation_id`, `method`, `endpoint`, `quota_class`, `priority`, `deadline_ms`, `result`, `stage`, `admission_cause`, `correlation`, `wire_sha256` |
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
| `admission_result` | `granted`, `rejected`, `cancelled`, `deadline`, `refused` (a mutation or session operation refused without waiting) |
| `admission_cause` | `window_full`, `daily_ceiling`, `modification_ceiling`, `no_standard_capacity`, `waiters_full`, `wait_expired` (why admission did not grant, §2.4; recorded on the operation span when it ends with an `Admission` error, or with a `Deadline` error because the operation deadline expired while a read or calculation waited for admission, where the cause is `wait_expired`) |
| `priority` | `standard`, `control` (the operation's admission lane, §2.1; a span field, not a metric label) |
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
| `manja_http_admission_wait_seconds` | histogram, s | `quota_class`, `admission_result` | 20 |
| `manja_http_retries_total` | counter, retries | `method`, `endpoint`, `error_class` | 496 |
| `manja_http_rejected_rows_total` | counter, rows | `endpoint` | 31 |
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
| `HttpDiagnostics` | active attempts, admission waiters in total and per quota class, recent failures (≤ `B-DIAG-01`) each with endpoint, method, status, error class, stage, broker error type, a redacted message (≤ `B-DIAG-02`) and a monotonic timestamp, the mutation dispatches in flight, the orphaned outcome ring (≤ `B-HTTP-14`) with the counts of orphaned outcomes and of outcomes the stream could not take (§2.17), and a snapshot revision |
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
| No dispatch permits | The permit API of 0.2 (`admit`, `DispatchPermit`, `PermitTarget`, the `*_with_permit` methods and `B-HTTP-12`) is removed; the mutation call is the admission boundary (§3.6) | A permit was a timed race whose validity bound had to outlast the caller's own checks, which is the caller's policy in the SDK's configuration; it held a window slot for a request that might never be sent, exactly the slot a cancellation needs under load; and it doubled the mutation surface. A call that is admitted now or refused with a hint gives the same affirmative not-sent evidence with no token |
| No wait for mutations | A mutation or session operation is admitted now or refused with a hint; only reads and calculations wait under `B-HTTP-06` (§2.4, §3.2, §3.6) | A wait between a caller's decision and the socket write makes that decision stale by construction, and a caller whose dispatch gate is serialised with other work cannot afford a call that sleeps. An immediate refusal is affirmative evidence that nothing was sent, and with the exact hint the caller can schedule better than a blind wait. Reads lose nothing by waiting, so they keep the bound |
| Detached mutations | A mutation or session operation is a plain function returning `Dispatched<T>`; its one attempt runs on a task the client owns and its outcome is delivered to the future if alive and always to the outcome stream, or to the orphan ring when no stream is registered; a refusal spawns nothing and emits no record; `Cancelled` is never produced for a mutation (§2.17, §3.2) | Once the bytes have left there is nothing to cancel, so a future that can be dropped promises a capability that does not exist, and a receipt the broker sent and the SDK read must not be lost to a `select!`, a timeout or a panic in the caller. A bounded stream that ends with an error rather than dropping silently is the ticker's pattern in this crate; diagnostics alone are evicted and would have to be polled; a callback would run caller code on the SDK's task, which §4.7 declines to defend against. The cost is one task spawn and one channel per mutation, at most ten a second by broker rule; reads, the volume path, pay nothing |
| Headroom | Admission capacity is readable as an advisory, local-view snapshot per class and lane, plus the modifications an order has left; a spent daily ceiling zeroes both lanes and announces no next slot, and a lane the reserve covers announces none either (§3.6) | A caller that can only learn capacity by being refused pays a refusal for every full window. A snapshot that agrees with the next decision lets it order its own work, and saying "advisory, local view" keeps the account-wide limit, which includes manual orders, where it belongs: with the caller. A reserving read would be the dispatch permit by another name |
| Session expiry | The 6 AM expiry is taken as IST, and a token expires at the next 06:00 IST strictly after its issue time: an issue at exactly 06:00:00 expires the next day, an issue before 06:00 expires that same morning (§2.2) | Kite's one sentence (`kite:user.md:115`) names no zone and says "the next day"; every documented Kite time is IST, and a caller that injects the issue time needs one boundary rule with no gap, so the next boundary after the instant is the reading the function implements |
| Ingress stamps | A response's `sent_at` is taken at dispatch, the last instant before the bytes leave the process, and `received_at` when the response head arrives, not when the body is complete; errors are stamped the same way, so a cut-off body still carries `received_at` (§2.18, §2.4) | A lost response is bounded by when the bytes left, and a response exists from the moment its head arrives, which is also when the error stage becomes `ResponseReceived`; stamping at the body's end would make a slow body look like a late response |
| Responses | `Response<T> { data: T, meta }` replaces the 0.2 envelope on every JSON endpoint; the `status` and `error_type` fields are gone and `data` is not optional (§2.18) | A success type with an `Option` inside makes every caller write a panic path or a fallback for a case the SDK promises cannot happen, and a fallback such as an empty order book is exactly the silent failure the crate exists to prevent. Fields that are constant on success are not data. Order rows keep every broker quantity and status field as reported so a caller can tell a partial fill from a terminal partially filled order; the SDK never collapses them |
| Wire forms | Every mutation renders a `WireForm` before it is sent and sends exactly those bytes; the forms of the two session operations exist but never render their token or checksum: the marker `<redacted>` stands in for each (§2.1) | An application that shows a person a compiled order and later journals what was sent needs the two to be provably the same bytes, and the hash of the form gives that without a second entry point. The session forms are included so every mutation has one; their bodies necessarily carry the documented token fields, so they render redacted and expose their bytes only through the hash and the send |
| Priority lanes | Two caller-selected lanes per quota class with a control reserve (`B-HTTP-15`, §2.1, §3.6); order cancellation and GTT deletion default to `Control`, and the mark is made per call through `HTTPClient::with_priority` rather than as a parameter on every method | Cancellation and the reads that confirm it share one transport with polling, and an in-flight request cannot be recalled, so a reserve that `Standard` traffic may never take is the only honest form of priority. Which reads are urgent is known only to the application, so the mark is per call and the SDK never decides it. A derived client keeps every method's signature unchanged |
| Oversized quote requests | Rejected before sending, never split or truncated | A split would return several snapshots taken at different times as one result |
| Endpoint support | Holdings auctions and the order charges calculation stay supported | Both are documented read or calculation endpoints with official fixtures |
| Holdings authorisation | Supported as the HTTP call that starts the flow, with the documented portal URL; the portal itself is left to the application | The endpoint and its result are documented (`kite:portfolio.md:503-541`). The remaining steps happen in the user's browser on the depository's portal, which an SDK cannot and should not drive |
| Order IDs | Checked types; a response ID that fails its grammar is a `Decode` error | An ID is a request path segment. A lenient fallback would let a malformed broker value reach a path unchecked, and every ID in the official samples already fits its grammar |
| Rejected list rows | The default equity order and trade list methods stay strict; a paired `*_with_rejections` method returns every row, rejected ones in place (§2.16) | Strict is the default because `?` keeps a failure loud, and an entirely rejected book can never pass for an empty one. The tolerant form lets a caller still see, and act on, the rows that decoded. Keeping partial data out of `HttpError` keeps broker rows out of commonly logged errors |
| Error details | A parse or decode failure is described by serde's category and position, with no field path | serde's message quotes the offending value. A field path would need a new dependency, and in a map-shaped payload such as a quote response its keys are broker values (instrument symbols). The tolerant order and trade lists hand the full serde error to the caller in `RowError`, outside any error (§2.16) |
| Mutual funds | Read-only: orders, SIPs, holdings and the instrument list | The documentation states that order placement cannot be done through the API, because it needs payment from the user's bank account (`kite:mutual-funds.md:3`), and lists only these reads (`kite:mutual-funds.md:5-11`). The official mocks still carry order and SIP placement, modification and cancellation responses, but no request for them is documented, so implementing them would mean inventing the request |
| GTT orders | Only LIMIT orders, each for the condition's instrument; a modification sends the complete trigger | The documentation lists `LIMIT` as the only order type and shows each order repeating the condition's exchange and tradingsymbol (`kite:gtt.md:56-64`); it recommends fetching the trigger and sending it back modified (`kite:gtt.md:390-393`) |
| Default features | `http`, `ticker` and `decoder` | Keeps every 0.1 import path available |
| Edition and minimum Rust version | Edition 2024; `rust-version` 1.88.0 | 1.88 is the first release with let chains, which the code uses. Every feature row, doc tests included, passes on 1.88.0, and 1.87.0 rejects the let chains (E0658). With `rust-version` at 1.88 the resolver picks dependency versions that build on it. Dependencies alone do not set the floor: down to 1.85 the resolver still finds compatible releases (the `icu_*` 2.3 crates, which need 1.88, fall back to earlier 2.x releases), and with the let chains reverted the all-features suite passes on 1.87.0 (checked on 2026-09-24) |
| 0.1 code | Removed in 0.2.0 rather than deprecated: the `WebSocketClient` ticker, `TickerRequest::subscribe_with_mode`, `kite::traits` and the error variants nothing constructs (`migration.md` §4) | 0.2.0 is itself the breaking release, so keeping code only 0.1 callers needed would carry it into a release that already requires migration. The old ticker read the socket directly, so it could not have been repaired without changing its behavior |
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

Each is versioned independently. The crate is pre-1.0: every intentional break ships only
in a minor-version bump and is listed in `migration.md`. Where practical, an item is
deprecated for a release before it is removed.
