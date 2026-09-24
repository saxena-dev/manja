//! An asynchronous Rust SDK for [Zerodha](https://zerodha.com/)'s
//! [Kite Connect](https://kite.trade/) API (version 3): the HTTP API for
//! accounts, orders, portfolios, market data and mutual funds, and the
//! WebSocket ticker for live market data and order updates.
//!
//! > **Manja** (IPA: /maːŋdʒʱaː/) n.: A type of abrasive string utilized primarily for flying fighter kites, especially prevalent in South Asian countries. It is crafted by coating cotton string with powdered glass or a similar abrasive substance.
//!
//! manja is built so that nothing goes wrong quietly. A failed call is never
//! reported as success, an order is never sent twice behind your back, a
//! ticker message is never dropped without an error, and credentials never
//! appear in logs or errors. When something does go wrong, the error tells you
//! what failed and how far the request got, so you can decide what to do next.
//!
//! # Where to start
//!
//! | To… | Start with |
//! |---|---|
//! | turn a login into credentials | [`Session::exchange`](kite::connect::api::Session::exchange) |
//! | call the HTTP API | [`HTTPClient`](kite::connect::client::HTTPClient) |
//! | place, modify or cancel orders | [`Orders`](kite::connect::api::Orders) and [`PlaceOrderRequest`](kite::connect::models::PlaceOrderRequest) |
//! | stream live market data | [`TickerBuilder`](kite::ticker::actor::owner::TickerBuilder) |
//! | decode ticker messages as they arrive | [`TypedEvents`](kite::ticker::typed::TypedEvents) |
//! | decode captured bytes offline | [`kite::decoder`] |
//! | handle failures | [`ManjaError`](kite::error::ManjaError) and [`HttpError`](kite::error::HttpError) |
//! | change deadlines, retries or rate limits | [`Config`](kite::connect::config::Config), [`SchedulerLimits`](kite::connect::scheduler::SchedulerLimits) and [`Admission`](kite::connect::admission::Admission) |
//! | collect metrics and traces | [`Observability`](kite::obs::Observability) |
//!
//! # A first program
//!
//! Credentials come from your own login flow: the user logs in on Kite's
//! page, you exchange the request token Kite hands back, and you keep the
//! resulting access token wherever your application keeps secrets. With them,
//! one client serves the whole HTTP API, and one ticker streams market data.
//!
//! ```no_run
//! # #[cfg(all(feature = "http", feature = "ticker"))]
//! # mod example {
//! use futures_util::StreamExt;
//! use manja::kite::connect::client::HTTPClient;
//! use manja::kite::connect::config::Config;
//! use manja::kite::connect::credentials::Credentials;
//! use manja::kite::protocol::InstrumentToken;
//! use manja::kite::ticker::actor::owner::{TickerBuilder, TickerEvent};
//! use manja::kite::ticker::Mode;
//!
//! # pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let credentials = Credentials::new("api_key", "access_token")?;
//!
//! // HTTP: create one client and share clones of it.
//! let client = HTTPClient::new(Config::default())?.with_credentials(credentials.clone());
//! let profile = client.user().profile().await?.data.expect("data");
//! println!("logged in as {}", profile.user_name);
//!
//! // Ticker: one background task owns the connection; you hold the handle.
//! let (handle, mut events, guard) = TickerBuilder::new(credentials).spawn()?;
//! handle.subscribe([InstrumentToken::new(408065)], Mode::Full).await?;
//! while let Some(event) = events.next().await {
//!     match event? {
//!         TickerEvent::Raw(message) => println!("{} bytes", message.payload().len()),
//!         TickerEvent::Lifecycle(change) => println!("{:?}", change.kind()),
//!         _ => {} // TickerEvent is non-exhaustive.
//!     }
//! }
//! println!("ticker ended: {:?}", guard.join().await);
//! # Ok(()) }
//! # }
//! ```
//!
//! The [README](https://github.com/saxena-dev/manja#readme) walks through
//! logging in, placing an order safely, decoding prices and handling errors,
//! step by step.
//!
//! # Features
//!
//! | Feature | What it adds | Default |
//! |---|---|---|
//! | `http` | [`HTTPClient`](kite::connect::client::HTTPClient) and every HTTP endpoint | yes |
//! | `ticker` | the supervised WebSocket ticker in [`kite::ticker`] | yes |
//! | `decoder` | decoding of binary and text ticker messages in [`kite::decoder`] | yes |
//!
//! Credentials, response models, errors, the protocol types in
//! [`kite::protocol`], observation envelopes and the observability types are
//! always available and need no async runtime. With
//! both `ticker` and `decoder`, [`kite::ticker::typed`] decodes messages as
//! they arrive. A build with only `decoder`, or no features at all, has no
//! Tokio or network dependency.
//!
//! # What you can rely on
//!
//! - **Success means success.** A call succeeds only when Kite answers with a
//!   2xx status, a `success` envelope and data of the expected shape.
//! - **Mutations are sent once.** Every call that can change your account
//!   (orders, position conversions, GTT changes, holdings authorisation and
//!   the session calls) makes exactly one attempt. Reads and margin and
//!   charge calculations retry transient failures on their own.
//! - **Rate limits are enforced locally**, at the rates Kite documents. A
//!   request waits briefly for capacity (`B-HTTP-06`, 5 seconds by default)
//!   and is refused with an `Admission` error rather than sent to be
//!   rejected by Kite.
//! - **Everything is bounded.** Queues, bodies, payloads and waits have
//!   documented defaults and limits, and exceeding one is an explicit error.
//! - **Unknown values are preserved** as unknown, never coerced into a known
//!   one and never sent back to Kite.
//!
//! # What manja leaves to you
//!
//! It never logs in on your behalf, and it never stores, refreshes or
//! invalidates credentials on its own. It persists nothing, and it does not
//! judge whether market data is fresh or whether an order filled: it reports
//! what Kite said and when. It installs no global `tracing` subscriber or
//! metrics exporter.
//!
//! # Reading these docs
//!
//! The item documentation uses three kinds of reference:
//!
//! - **`docs/contract.md §N`** is a section of the
//!   [behavioral contract](https://github.com/saxena-dev/manja/blob/main/docs/contract.md),
//!   which specifies every capability, bound and decision in detail.
//! - **Bound IDs** such as `B-HTTP-01` name a runtime limit. Each bound's
//!   default, range and meaning are listed in `docs/contract.md` §3,
//!   [Runtime bounds](https://github.com/saxena-dev/manja/blob/main/docs/contract.md#3-runtime-bounds).
//! - **`kite:<page>.md:<lines>`** cites the Kite Connect documentation: the
//!   page at `https://kite.trade/docs/connect/v3/<page>/`, at the given lines
//!   of its Markdown source. For example, `kite:orders.md` is
//!   [the orders page](https://kite.trade/docs/connect/v3/orders/). The exact
//!   version each citation refers to is recorded, with its SHA-256, in
//!   [`docs/kite-sources.toml`](https://github.com/saxena-dev/manja/blob/main/docs/kite-sources.toml).
//!
//! # Status and disclaimer
//!
//! manja is pre-1.0 and still changing. Breaking changes ship only in
//! minor-version bumps, each listed in the
//! [migration guide](https://github.com/saxena-dev/manja/blob/main/docs/migration.md),
//! and where practical an item is deprecated for a release before it is
//! removed. manja is an independent open-source project: it is not affiliated
//! with Zerodha in any way, and Zerodha neither makes, endorses nor supports
//! it.
//!
//! The software is provided "as is", without warranty of any kind. The author
//! and contributors take no responsibility for any financial losses, damages
//! or other issues arising from its use. Nothing in this crate is a statement
//! that data is fresh, complete or fit for trading.
#![warn(rust_2018_idioms)]

pub mod kite;

// The README's examples compile as doc tests, so they cannot drift from the
// API. They use every feature, so they are checked when all are enabled.
#[cfg(all(doctest, feature = "http", feature = "ticker", feature = "decoder"))]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
