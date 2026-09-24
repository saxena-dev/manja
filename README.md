manja
=====

[![CI](https://github.com/saxena-dev/manja/actions/workflows/ci.yml/badge.svg)](https://github.com/saxena-dev/manja/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/gh/saxena-dev/manja/graph/badge.svg)](https://codecov.io/gh/saxena-dev/manja)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)
[![Docs.rs](https://docs.rs/manja/badge.svg)](https://docs.rs/manja)
[![Crates.io](https://img.shields.io/crates/v/manja.svg)](https://crates.io/crates/manja)
[![Downloads](https://img.shields.io/crates/d/manja.svg)](https://crates.io/crates/manja)

**manja** is an asynchronous Rust SDK for [Zerodha](https://zerodha.com/)'s
[Kite Connect](https://kite.trade/) API (version 3): the HTTP API for accounts, orders,
portfolios, market data and mutual funds, and the WebSocket ticker for live market data
and order updates.

> **Manja** (IPA: /maːŋdʒʱaː/) n.: A type of abrasive string utilized primarily for flying fighter kites, especially prevalent in South Asian countries. It is crafted by coating cotton string with powdered glass or a similar abrasive substance.

When software talks to a broker, the expensive bugs are rarely crashes. They are the
quiet ones: an error that looked like an empty order book, an order placed twice because
a timeout was retried, a dropped tick nobody noticed, an access token printed into a log
file. manja is built so that these cannot happen quietly. When something goes wrong, you
are told, with enough detail to decide what to do next.

## Why manja

- **Failures are never disguised as success.** A response counts as a success only if
  the status is 2xx, Kite's envelope (where the endpoint has one) says `success`, and
  the data matches the endpoint's type. Anything else is an error that says what
  failed, where, and how far the request got.
- **Orders are never sent twice behind your back.** Every call that can change your
  account (orders, position conversions, GTT changes, holdings authorisation and the
  session calls) makes exactly one attempt. If a response is lost, the error says Kite
  *may* have received the request, so you can check before trying again. Reads and
  margin and charge calculations, which are safe to repeat, retry transient failures on
  their own.
- **Kite's rate limits are enforced before you hit them.** Every documented limit (for
  example 10 orders a second, 400 a minute, 5000 a day, and 25 modifications per order)
  is applied locally. A request waits briefly for capacity, 5 seconds by default, and
  is refused with an `Admission` error rather than sent to be rejected by Kite.
- **Secrets stay secret.** API keys, access tokens and secrets never appear in `Debug`
  output, errors, diagnostics, spans or metric labels.
- **Nothing grows without bound.** Every queue, body, payload and wait has a documented
  default and limit. Exceeding one is an explicit error, not a slow leak.
- **The ticker never drops data silently.** Every message it accepts is delivered in the
  order it arrived, and anything it can't accept, such as an oversized message, is
  reported rather than skipped. Reconnects restore your subscriptions before reporting
  the connection as active, and a consumer that falls behind gets an error, not a gap.
- **Unknown values are kept, not guessed.** When Kite adds a new order status, exchange
  or message type, you receive it as an unknown value instead of a parse failure or a
  wrong guess.
- **You own your telemetry.** manja installs nothing globally. Plug in your own
  `tracing` subscriber and metrics recorder, or none at all.
- **Take only what you need.** The HTTP client, the ticker and the decoder are separate
  features. A decoder-only build has no Tokio or network dependency.

## Installation

```toml
[dependencies]
manja = "0.2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

All three features (`http`, `ticker` and `decoder`) are on by default. To depend on
less, turn off the defaults and pick what you use:

```toml
manja = { version = "0.2", default-features = false, features = ["http"] }
```

manja needs **Rust 1.88** or newer. The HTTP client and the ticker run on the Tokio
runtime you already use, multi-threaded or not: `rt` instead of `rt-multi-thread` is
enough for a single-threaded one.

## Getting started

### Before you start

You need a Zerodha trading account, and a Kite Connect app created on the
[Kite Connect developer console](https://developers.kite.trade/), which gives you an
**API key** and an **API secret**. The API key itself is free. Zerodha charges
separately for some services, and the
[Kite Connect documentation](https://kite.trade/docs/connect/v3/) has the current
details.

### 1. Get an access token

Kite Connect authenticates with an **access token** that you obtain when the user logs
in. The user signs in on Kite's own login page, and Kite redirects them to your app with
a **request token** ([Kite's login flow](https://kite.trade/docs/connect/v3/user/)).
manja never drives that page for you, because a login is between the user and Kite. Once
you have the request token, exchange it for a session:

```rust,no_run
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::{ApiKey, ApiSecret, RequestToken};

async fn log_in(request_token: &str, api_secret: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = HTTPClient::new(Config::default())?;
    let session = client
        .session(ApiKey::new("your_api_key")?)
        .exchange(&RequestToken::new(request_token)?, &ApiSecret::new(api_secret)?)
        .await?
        .data
        .expect("a successful exchange carries the session");

    // The API key and access token, ready for every other call.
    let credentials = session.credentials()?;
    // Storing them is up to you: manja keeps nothing on disk.
    Ok(())
}
```

The API secret is only borrowed for that one call, to compute the checksum Kite asks
for. The exchange is never retried automatically.

An access token lasts until 6 AM the next day, unless it is invalidated sooner, so plan
for the user to log in again each trading day.

### 2. Call the HTTP API

Give the client your credentials, then reach each part of the API through it:

```rust,no_run
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::Credentials;

async fn account(credentials: Credentials) -> Result<(), Box<dyn std::error::Error>> {
    let client = HTTPClient::new(Config::default())?.with_credentials(credentials);

    let profile = client.user().profile().await?.data.expect("data");
    println!("hello, {}", profile.user_name);

    let holdings = client.portfolio().get_holdings().await?.data.unwrap_or_default();
    let positions = client.portfolio().get_positions().await?.data.expect("data");
    println!("{} holdings, {} open positions", holdings.len(), positions.net.len());
    Ok(())
}
```

Every JSON call returns a `KiteApiResponse`, Kite's own envelope. When the call
succeeds, its `data` is always present, so `expect` documents a guarantee rather than a
hope. For lists, `unwrap_or_default()` reads just as well, and an empty list is a
perfectly good fallback.

An `HTTPClient` is cheap to clone. Clones share one connection pool and one set of rate
limits, so create it once and hand clones to the tasks that need it.

### 3. Place an order, carefully

Orders are built from a request type that is checked before anything is sent. A
quantity of zero, a LIMIT order without a price or a malformed tag is a `Validation`
error, and nothing reaches Kite.

```rust,no_run
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::models::{
    Exchange, OrderType, OrderVariety, PlaceOrderRequest, ProductType, TransactionType,
};
use manja::kite::protocol::Quantity;

async fn buy(client: &HTTPClient) -> Result<(), Box<dyn std::error::Error>> {
    let mut order = PlaceOrderRequest::new(
        OrderVariety::Regular,
        Exchange::NSE,
        "INFY",
        TransactionType::BUY,
        OrderType::Limit,
        Quantity::new(1)?,
        ProductType::CashAndCarry,
    );
    order.price = Some(1500.0);
    // A tag lets you find this order again if the response is lost.
    order.tag = Some("rebalance42".into());

    match client.orders().place_order(&order).await {
        Ok(receipt) => println!("accepted as {}", receipt.data.expect("data").order_id),
        Err(err) if err.as_http().is_some_and(|e| e.may_have_reached_broker()) => {
            // The request left the process, so the order may exist. Check the
            // order book before deciding to place it again.
            let book = client.orders().list_orders().await?.data.unwrap_or_default();
            let placed = book.iter().any(|o| o.tag.as_deref() == Some("rebalance42"));
            println!("order was placed: {placed}");
        }
        Err(err) => return Err(err.into()),
    }
    Ok(())
}
```

A receipt means Kite accepted the order, not that it filled. Follow its progress with
`get_order_history`, or with the order updates the ticker delivers.

When you want a last check between waiting for rate-limit capacity and sending, reserve
the capacity first with `HTTPClient::admit` and place the order with
`place_order_with_permit`. The permit expires after a second, which keeps the check
close to the send.

### 4. Stream live market data

The ticker is a single background task that owns the WebSocket connection. You talk to
it through a handle and read everything it receives from one stream:

```rust,no_run
use futures_util::StreamExt;
use manja::kite::connect::credentials::Credentials;
use manja::kite::protocol::InstrumentToken;
use manja::kite::ticker::Mode;
use manja::kite::ticker::actor::owner::{TickerBuilder, TickerEvent};

async fn watch(credentials: Credentials) -> Result<(), Box<dyn std::error::Error>> {
    let (handle, mut events, guard) = TickerBuilder::new(credentials).spawn()?;
    handle.subscribe([InstrumentToken::new(408065)], Mode::Full).await?;

    while let Some(event) = events.next().await {
        match event? {
            TickerEvent::Raw(message) => println!("{} bytes", message.payload().len()),
            TickerEvent::Lifecycle(change) => println!("{:?}", change.kind()),
            _ => {} // TickerEvent is non-exhaustive: later versions may add events.
        }
    }
    println!("ticker ended: {:?}", guard.join().await);
    Ok(())
}
```

A few things are worth knowing:

- **Subscriptions survive reconnects.** The ticker remembers what you asked for and
  sends it again on every new connection before it reports `Active`.
- **The stream tells you when it ends and why.** It ends with `None` only after you call
  `handle.shutdown()`. Any other ending, such as a rejected access token or reconnect
  attempts running out, yields one error first.
- **Read promptly.** Delivery is bounded. If your consumer falls too far behind, the
  ticker stops with an explicit error rather than dropping messages you never saw.
- **`Active` is not freshness.** It means your subscriptions were written to the
  connection. Whether a price is recent enough for your purpose is your call.
- **Keep a handle.** Dropping the last `TickerHandle` stops the ticker. If you spawn it
  in a helper, return the handle along with the stream.

`handle.status()` gives you a snapshot at any time: the connection state, how long ago
the last message and heartbeat arrived, the queue's depth, the failures that led to
reconnects and, once the ticker has stopped, why.

### 5. Decode prices and order updates

Ticker messages arrive as raw bytes, exactly as Kite sent them. Wrap the stream in
`TypedEvents` to receive each message together with its decoding:

```rust,no_run
use futures_util::StreamExt;
use manja::kite::decoder::adapter::{Adapter, DecodedEvent};
use manja::kite::decoder::packets::{Packet, scaled};
use manja::kite::obs::Observability;
use manja::kite::obs::schema::SourceMode;
use manja::kite::protocol::scale::Segment;
use manja::kite::ticker::actor::owner::TickerEvents;
use manja::kite::ticker::typed::{TypedEvent, TypedEvents};

async fn prices(events: TickerEvents) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = Adapter::new(SourceMode::Live, &Observability::disabled());
    let mut typed = TypedEvents::new(events, adapter);
    while let Some(item) = typed.next().await {
        let TypedEvent::Observation { decoded, .. } = item? else { continue };
        for event in decoded?.events {
            if let DecodedEvent::Packet { packet: Packet::Full(p), .. } = event {
                let price = scaled(p.fields.last_price, Segment::Nse)?.to_f64();
                println!("{}: {price}", p.fields.instrument_token.get());
            }
        }
    }
    Ok(())
}
```

Prices on the wire are integers. Kite divides NSE currency derivatives by 10,000,000 and
everything else by 100, so `scaled` asks you for the segment rather than guessing it.
The BSE currency segment has no verified scale, so `scaled` refuses it rather than
guess.

The decoder works on plain bytes and needs no runtime, so it is just as useful for
replaying captured data offline. With `default-features = false, features = ["decoder"]`
it builds with no network stack at all.

## Handling errors

Every failed HTTP call is a `ManjaError::Http(HttpError)`. The error's **kind** tells you
what happened, and its **stage** tells you how far the request got:

```rust,no_run
use manja::kite::connect::client::HTTPClient;
use manja::kite::error::HttpErrorKind;

async fn holdings(client: &HTTPClient) {
    match client.portfolio().get_holdings().await {
        Ok(response) => println!("{} holdings", response.data.unwrap_or_default().len()),
        Err(err) => match err.as_http() {
            Some(e) if e.kind() == HttpErrorKind::AuthRejected => {
                // The access token expired or was invalidated: log in again.
            }
            Some(e) => eprintln!("{e} (may have reached Kite: {})", e.may_have_reached_broker()),
            None => eprintln!("{err}"),
        },
    }
}
```

| `HttpErrorKind` | What it means |
|---|---|
| `Validation` | your request was invalid; nothing was sent |
| `Configuration` | the client or the request could not be built; nothing was sent |
| `Admission` | a local limit refused the request: rate-limit capacity was not available within the wait, the day's order limit or an order's modification limit is spent, or a dispatch permit does not match; nothing was sent |
| `Deadline` | the operation's deadline, or a dispatch permit, expired before an attempt could start; nothing was sent |
| `Transport` | the network failed or timed out; a deadline that runs out during an attempt shows up here, as a timeout |
| `Broker` | Kite returned an error, available through `e.broker()` |
| `AuthRejected` | Kite rejected the credentials |
| `HttpStatus` | an error status without Kite's error envelope |
| `Decode` | a response arrived but could not be understood |
| `Cancelled` | never returned by a call: it appears only in `HTTPClient::diagnostics()`, for an operation whose future was dropped |

The stage is `NotStarted` only when manja knows for certain that the request never left
your machine. Anything later means Kite may have acted on it, and
`may_have_reached_broker()` says so in one call. An error's detail never includes
values decoded from the response body. A `Broker` error does carry Kite's own message,
as Kite sent it, which can mention amounts or IDs.

## Cancellation and timeouts

Every call is an ordinary future, so `tokio::select!` and `tokio::time::timeout` work as
you'd expect, with one thing to keep in mind. A future does nothing until it is first
polled, and dropping it before it sends anything cancels only local work. Dropping it
after the request was sent does not cancel anything at Kite: the order may still be
placed. Treat a cancelled order call like a lost response, and check the order book.

The same holds for ticker commands. Once a command has been handed to the ticker,
dropping the future that sent it does not withdraw it; `desired_revision` in
`handle.status()` shows whether it was applied.

## Rate limits, retries and deadlines

You don't need to configure anything to stay within Kite's documented limits. By
default every request class is admitted at its documented rate, a request waits up to 5
seconds for capacity, a read makes up to three attempts with jittered backoff between
them, and each operation has 30 seconds to finish (15 for the session calls). The daily
order limit and the per-order modification limit never wait: once spent, they refuse
the request straight away. `docs/contract.md` calls these limits quotas, and lists every
bound with its range.

When your application needs different bounds, set them once on the configuration.
Every setter checks its value against a documented range:

```rust,no_run
use std::time::Duration;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::scheduler::SchedulerLimits;

fn client() -> Result<HTTPClient, Box<dyn std::error::Error>> {
    let scheduler = SchedulerLimits::default()
        .with_operation_deadline(Duration::from_secs(10))?
        .with_read_attempts(2)?;
    let config = Config::default().with_limits(HttpLimits::default().with_scheduler(scheduler));
    Ok(HTTPClient::new(config)?)
}
```

Kite applies its limits per user and API key, so everything using the same key should
share one admission scope. Clones of a client already do. For clients built separately,
pass the same scope to each: `HTTPClient::builder(config).admission(shared.clone())`.

## Observability

Telemetry is off by default, and recording nothing allocates nothing. To turn it on,
give the client or ticker an `Observability` handle built around a metrics recorder,
and install your own `tracing` subscriber if you want spans:

```rust,no_run
use std::sync::Arc;
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::obs::{InMemoryRecorder, Observability};

fn client() -> Result<HTTPClient, Box<dyn std::error::Error>> {
    let recorder = Arc::new(InMemoryRecorder::new());
    Ok(HTTPClient::with_observability(Config::default(), Observability::with_recorder(recorder))?)
}
```

Implement `MetricRecorder` to feed Prometheus, OpenTelemetry or anything else. Metric
names and labels form a fixed, versioned schema with small, closed label sets, so they
won't blow up your metrics backend. Even with no collector at all,
`HTTPClient::diagnostics()` and the ticker's `status()` answer "what is it doing right
now?" in code.

## What manja does not do

Knowing the edges saves surprises:

- It does not log in for you, and it does not store, refresh or invalidate credentials
  on its own.
- It does not decide whether market data is fresh enough, or whether an order filled.
  It tells you what Kite said and when.
- It does not persist anything: no caches, no files, no databases.
- It does not place or change mutual fund orders or SIPs.
  [Kite's documentation](https://kite.trade/docs/connect/v3/mutual-funds/) says mutual
  fund orders can't be placed through the API, because they need payment from the
  user's bank account, so manja offers only the reads.
- It never retries a request that could change your account.
- It does not cover alerts, the holdings summary, the full profile, the trigger range,
  or receiving postbacks over HTTP. The ticker delivers order updates instead.

## How manja is tested

Trust has to be earned, so here is how manja tries to earn it:

- **Official responses.** The HTTP tests run against Zerodha's own
  [`kiteconnect-mocks`](https://github.com/zerodha/kiteconnect-mocks), pinned to a
  commit and served byte for byte. Expected values are written out by hand, never
  produced by parsing the fixture a second time.
- **Real ticker bytes.** The decoder is checked against a capture from a live Kite
  session and a vendored corpus of framed, index, heartbeat and malformed messages,
  each with its origin and SHA-256 recorded.
- **Hostile inputs.** Seeded property campaigns feed the decoder more than 70,000
  generated, mutated and truncated inputs, checking that it stays within its bounds and
  that every decoded packet re-encodes to its exact bytes.
- **Nothing leaves your machine.** Every test runs against loopback servers. No test
  needs a Kite account or touches the network.
- **Measured overhead.** A benchmark with budgets, run on demand, measured metrics and
  tracing adding at most 1.17× to an operation's median time (the budget is 1.25×),
  decoding allocating nothing per message, and ticker delivery at 282 µs at p99.9 (the
  budget is 1 ms). Those figures come from an Apple M5 running macOS 26.6.2 and Rust
  1.98; your hardware will differ.
- **Every claim has a source.** Behavior that follows Kite's documentation cites the
  exact page and lines, and those pages are recorded with their SHA-256, so a change in
  Kite's documentation can be detected.
- **Every configuration is tested.** CI runs the tests in eight feature configurations,
  from no features to all three, on Rust 1.88 and on stable, and checks that a build
  without `http` or `ticker` pulls in no network stack.

The full details are in [`docs/verification.md`](docs/verification.md).

## Examples

The HTTP examples run against a local server that serves Zerodha's official mock
responses, the ticker examples against a local WebSocket that sends recorded and
hand-built ticker messages, and the decoding example needs no server at all. So you can try everything
without an account:

| Example | What it shows |
|---|---|
| `http_client` | account reads, placing an order with a permit, quotes, diagnostics and metrics |
| `session` | exchanging a request token and invalidating a session |
| `ticker` | subscribing, changing modes, reading raw messages, status and a clean shutdown |
| `ticker_typed` | the ticker with the decoder: every message with its decoded packets |
| `decode_offline` | decoding captured ticker bytes with no runtime or network |

```text
cargo run --example http_client
cargo run --example decode_offline --no-default-features --features decoder
```

## Supported Kite Connect endpoints

- **Session**: token exchange and invalidation
- **User**: profile, and funds and margins, overall or by segment
- **Orders**: place, modify and cancel; the order book, an order's history, trades, and
  an order's trades
- **GTT**: place, modify, delete, list and fetch Good Till Triggered orders
- **Portfolio**: holdings, positions, position conversion, holdings auctions, and
  starting holdings authorisation at the depository
- **Market data**: instruments, full quotes, OHLC and last price
- **Historical data**: candles for every documented interval, with open interest
- **Mutual funds**: orders, SIPs, holdings and the fund list (read-only)
- **Margins and charges**: order and basket margins, and order charges
- **WebSocket**: LTP, quote, full and index packets with market depth, order updates,
  and error and informational messages

The exact paths and their behavior are in `docs/contract.md` §2.1.

## Feature flags

| Feature | What it adds | Default |
|---|---|---|
| `http` | `HTTPClient` and every HTTP endpoint | yes |
| `ticker` | the supervised WebSocket ticker | yes |
| `decoder` | decoding of binary and text ticker messages | yes |

With both `ticker` and `decoder`, `kite::ticker::typed` decodes messages as they
arrive. Credentials, models, errors, the protocol types, envelopes and the
observability types are always available.

## Documentation

- [API reference on docs.rs](https://docs.rs/manja)
- [`docs/contract.md`](docs/contract.md): every capability, bound and decision in
  detail. The API reference cites it by section, and cites limits by IDs such as
  `B-HTTP-01`.
- [`docs/verification.md`](docs/verification.md): how each behavior is tested.
- [`docs/migration.md`](docs/migration.md): upgrading from 0.1.

## Upgrading from 0.1

0.2 is a large, deliberate break. The browser login is gone, errors are never reported
as success, orders take dedicated request types, and `TickerBuilder` replaces the old
WebSocket client. [`docs/migration.md`](docs/migration.md) lists every change with what
to do about it.

## Status and disclaimer

manja is pre-1.0 and still changing. Breaking changes ship only in minor-version bumps,
each listed in the [migration guide](docs/migration.md). Where practical, an item is
deprecated for a release before it is removed.

manja is an independent open-source project. It is not affiliated with Zerodha in any
way, and Zerodha neither makes, endorses nor supports it. For questions and bug reports,
please [open an issue](https://github.com/saxena-dev/manja/issues).

The software is provided "as is", without warranty of any kind. The author and
contributors take no responsibility for any financial losses, damages or other issues
arising from its use. Nothing in this crate is a statement that data is fresh, complete
or fit for trading. Test against your own requirements before trading real money.

## License

[MIT](LICENSE)
