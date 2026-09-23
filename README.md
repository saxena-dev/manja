manja
=====

[![CI](https://github.com/saxena-dev/manja/actions/workflows/ci.yml/badge.svg)](https://github.com/saxena-dev/manja/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.95+](https://img.shields.io/badge/rust-1.95%2B-orange.svg)](https://www.rust-lang.org/)
[![Docs.rs](https://docs.rs/manja/badge.svg)](https://docs.rs/manja)
[![Crates.io](https://img.shields.io/crates/v/manja.svg)](https://crates.io/crates/manja)
[![Downloads](https://img.shields.io/crates/d/manja.svg)](https://crates.io/crates/manja)

> **Manja** (IPA: /maːŋdʒʱaː/) n.: A type of abrasive string utilized primarily for flying fighter kites, especially prevalent in South Asian countries. It is crafted by coating cotton string with powdered glass or a similar abrasive substance.

An asynchronous Rust client library for [Zerodha](https://zerodha.com/)'s
[Kite Connect](https://kite.trade/) HTTP and WebSocket APIs.

## Features and MSRV

| Feature | What it adds | Default |
|---|---|---|
| `http` | `HTTPClient` and its resources: session, user, orders, GTT, portfolio, market, margins and charges | yes |
| `ticker` | the supervised single-owner WebSocket ticker, plus the deprecated legacy client | yes |
| `decoder` | pure, bounded decoding of binary and text ticker messages, and a provenance adapter | yes |

With no features, the crate is the common slice only (credentials, models, envelopes,
observability types, protocol types): no Tokio, no network stack. `http` does not pull in
the WebSocket stack, and `ticker` pulls in neither the HTTP stack nor the decoder.
`ticker` and `decoder` together enable `kite::ticker::typed`.

The minimum supported Rust version is **1.95.0**.

## Quick start

```rust
use futures_util::StreamExt;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::Credentials;
use manja::kite::protocol::InstrumentToken;
use manja::kite::ticker::actor::owner::{TickerBuilder, TickerEvent};
use manja::kite::ticker::Mode;

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let credentials = Credentials::new("api_key", "access_token")?;

    let client = HTTPClient::new(Config::default())?.with_credentials(credentials.clone());
    let _profile = client.user().profile().await?;

    let (handle, mut events, guard) = TickerBuilder::new(credentials).spawn()?;
    handle.subscribe([InstrumentToken::new(408065)], Mode::Full).await?;
    while let Some(item) = events.next().await {
        if let TickerEvent::Raw(observation) = item? {
            let _bytes = observation.payload().as_bytes();
        }
    }
    guard.join().await;
    Ok(())
}
```

## Examples

Every example runs against loopback servers that serve the official
[`kiteconnect-mocks`](https://github.com/zerodha/kiteconnect-mocks) responses or the
vendored ticker bytes. None contacts a live endpoint unless you ask it to with inputs you
supply.

| Example | Shows | Features |
|---|---|---|
| `http_client` | profile, funds, holdings, positions and orders; order construction and placement with a dispatch permit; quotes with missing keys; diagnostics; a host-owned tracing formatter and in-memory metrics | `http` |
| `session` | pre-session token exchange and invalidation without a secret; `live` mode only with supplied inputs (secret on stdin) | `http` |
| `ticker` | raw heartbeat, binary and text delivery; subscribe, set mode, unsubscribe; revisions and lifecycle; status; clean shutdown | `ticker` |
| `ticker_typed` | the ticker composed with the decoder: each observation with its decoded packets | `ticker`, `decoder` |
| `decode_offline` | framing, packet decoding and provenance over a captured file, with no runtime | `decoder` |

```text
cargo run --example http_client
cargo run --example decode_offline --no-default-features --features decoder
```

## What the SDK guarantees, and what it does not

**Credentials.** A client or ticker holds an immutable `Credentials` snapshot (API key
and access token) that you supply. The SDK never logs in, stores, refreshes or
invalidates credentials on its own. Token exchange borrows the API secret for one call;
invalidation takes no secret. Tokens and secrets never appear in `Debug`, errors,
diagnostics, spans or metric labels.

**HTTP responses.** Success requires a 2xx status and a `status: "success"` envelope
whose data matches the endpoint's type. Anything else is an `HttpError` with a
category, the endpoint template, the HTTP status and broker error type if any, the
attempt number, and **stage evidence**: `NotStarted` only when the SDK knows the request
never left the process, `Started` once the broker may have received it.

**Retries, admission and permits.** Reads and margin calculations retry transient
failures (429, 502–504, transport faults, attempt timeouts) with capped, jittered backoff
within a total deadline. Order placement, modification, cancellation, position
conversion, GTT placement, modification and deletion, and the session operations make
**exactly one attempt**: a lost response is reported, never retried or assumed. Admission enforces the documented quotas (quote 1/s; orders 10/s,
400/min, 5000/day; 25 modifications per order; others 10/s). A `DispatchPermit` from
`HTTPClient::admit` reserves capacity for one specific order operation, expires after one
second, and is consumed by use.

**Units and values.** Order and quote models use the broker's JSON numbers. Ticker
prices are raw `int32` integers; convert with the segment you supply (currencies ÷ 10⁷,
others ÷ 100; the BSE currency segment has no verified scale and is refused). Broker
datetimes without an offset are IST. Values the SDK does not know (a new order status,
exchange or text message type) are preserved as `Inbound::Unknown`, never coerced, and
never usable as outbound values. Optional broker fields are `Option`s; an empty refresh
token is `None`.

**Bounds.** Every queue, body, payload, count and wait has a documented default, minimum
and maximum (`HttpLimits`, `SchedulerLimits`, `AdmissionLimits`, `TickerLimits`,
`ReconnectLimits`, `FramingLimits`, `TextLimits`). Exceeding one is an explicit error.

**Ticker ownership and delivery.** One owner task holds the socket, epochs, sequence and
termination; you get a `TickerHandle` (clone, send, sync), the one primary
`TickerEvents` receiver, and a `TaskGuard`. Every binary and text message, heartbeats
included, is delivered raw before any decoding, in source order with lifecycle events.
The queue is bounded by messages, retained bytes, payload size, oldest age and delivery
wait; a consumer that falls behind ends delivery with an explicit error rather than a
silent drop. Reconnects are bounded, get fresh epochs, report gap facts without
backfilling, and restore the desired subscriptions (subscribe, then mode) before
`Active`. A 401 or 403 handshake stops the ticker. The stream ends with `None` only after
a clean shutdown; any other end yields one error first.

**Acceptance, acknowledgement, fill and freshness.** A subscription command completes
when the owner accepts it, with a revision; `CommandsSent` means it was written to the
socket, not that the broker acted on it. `Active` means the desired map was written to a
connection, not that quotes are current. An order receipt means the broker accepted the
request, not that it filled; a GTT receipt names the trigger, not that it fired. Whether market data is current is yours to decide.

**Cancellation and concurrency.** Futures are lazy: dropping one before its first poll
does nothing. Dropping an HTTP future after dispatch does not cancel anything at the
broker. Dropping a ticker command future after its first poll does not withdraw an
accepted command. Clients are cheap to clone and share one transport and admission scope
with no client-wide lock.

**Observability.** Nothing is installed globally. Pass an `Observability` handle with
your own `MetricRecorder` (or none), and your own `tracing` subscriber; spans and metrics
use closed label domains only. `HTTPClient::diagnostics` and `TickerHandle::status` work
with no collector at all.

## Documentation

- [`docs/contract.md`](docs/contract.md): capabilities, runtime bounds with their
  defaults and ranges, the observability schema, and the decisions behind them.
- [`docs/verification.md`](docs/verification.md): fixtures, test targets, decoder
  qualification and observability budgets.
- [`docs/migration.md`](docs/migration.md): changes from 0.1.
- [`docs/kite-sources.toml`](docs/kite-sources.toml): the Kite Connect documentation
  pages cited as `kite:<page>.md:<lines>`, with the time each was accessed and its
  SHA-256. `scripts/verify-kite-sources.sh` checks them against the live pages.

## Supported Kite Connect 3.0 endpoints

- **Session**: `POST /session/token` (exchange), `DELETE /session/token` (invalidate)
- **User**: `GET /user/profile`, `GET /user/margins`, `GET /user/margins/:segment`
- **Orders**: `POST /orders/:variety`, `PUT /orders/:variety/:order_id`,
  `DELETE /orders/:variety/:order_id`, `GET /orders`, `GET /orders/:order_id`,
  `GET /trades`, `GET /orders/:order_id/trades`
- **GTT**: `POST /gtt/triggers`, `GET /gtt/triggers`, `GET /gtt/triggers/:id`,
  `PUT /gtt/triggers/:id`, `DELETE /gtt/triggers/:id`
- **Portfolio**: `GET /portfolio/holdings`, `GET /portfolio/positions`,
  `PUT /portfolio/positions`, `GET /portfolio/holdings/auctions`
- **Market**: `GET /instruments`, `GET /instruments/:exchange`, `GET /quote`,
  `GET /quote/ohlc`, `GET /quote/ltp`
- **Margins and charges**: `POST /margins/orders`, `POST /margins/basket`,
  `POST /charges/orders`
- **WebSocket**: binary market data (LTP, quote, full and index packets), text order
  updates, errors and messages

Not supported: historical candles, mutual funds, and holdings authorisation.

## Migrating from 0.1

The browser login flow is removed, several response types and error behaviors were
corrected, and the legacy WebSocket client is deprecated.
[`docs/migration.md`](docs/migration.md) lists every change. The short version: obtain
the request token yourself, call `client.session(api_key).exchange(...)`, build
`Credentials` from the returned session, and use `TickerBuilder` instead of
`WebSocketClient`.

## Disclaimer

* `manja` is in development and should be considered unstable; its API may change.
* The software is provided "as-is" without any warranties, express or implied. The author
  and contributors take no responsibility for any financial losses, damages, or other
  issues that may arise from its use. Nothing in this crate is a statement that data is
  fresh, complete or fit for trading.
