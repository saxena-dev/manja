//! > **Manja** (IPA: /maːŋdʒʱaː/) n.: A type of abrasive string utilized primarily for flying fighter kites, especially prevalent in South Asian countries. It is crafted by coating cotton string with powdered glass or a similar abrasive substance.
//!
//! An asynchronous client library for [Zerodha](https://zerodha.com/)'s
//! [Kite Connect](https://kite.trade/) HTTP and WebSocket APIs.
//!
//! # Slices
//!
//! | Feature | Module | What it gives |
//! |---|---|---|
//! | `http` | [`kite::connect`] | [`HTTPClient`](kite::connect::client::HTTPClient) and its resources, with explicit credentials, admission against the documented quotas, bounded deadlines and one-attempt mutations, and total response classification |
//! | `ticker` | [`kite::ticker`] | a supervised single-owner ticker that delivers raw observations and lifecycle events in source order, restores its desired subscriptions on every bounded reconnect, and stops on credential rejection |
//! | `decoder` | [`kite::decoder`] | a pure, bounded parser of binary and text ticker messages, and a provenance adapter |
//!
//! [`kite::envelope`], [`kite::obs`], [`kite::protocol`] and
//! [`kite::error`] are always available. With `ticker` and `decoder`
//! together, [`kite::ticker::typed`] decodes the ticker's observations as
//! they pass, alongside the raw ones.
//!
//! What the SDK does not do: it never logs in on your behalf, stores or
//! refreshes credentials, persists observations, or judges whether market
//! data is current. `Active` means the desired subscriptions were
//! written to a connection, not that quotes are current. Delivery ends with a
//! terminal error, never silently.
//!
//! The behavior and bounds are specified in `docs/contract.md`, which the
//! documentation cites as `docs/contract.md §N` and by bound IDs such as
//! `B-HTTP-01`. Citations of the form `kite:<page>.md:<lines>` point into the
//! Kite Connect v3 documentation pages listed, with access time and SHA-256,
//! in `docs/kite-sources.toml`.
//!
//! # Example
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
//! // Credentials come from your own login flow and storage.
//! let credentials = Credentials::new("api_key", "access_token")?;
//!
//! // HTTP: one client, shared by clones.
//! let client = HTTPClient::new(Config::default())?.with_credentials(credentials.clone());
//! let profile = client.user().profile().await?;
//! let holdings = client.portfolio().get_holdings().await?;
//! # let _ = (profile, holdings);
//!
//! // Ticker: one owner task; commands and status through the handle.
//! let (handle, mut events, guard) = TickerBuilder::new(credentials).spawn()?;
//! handle.subscribe([InstrumentToken::new(408065)], Mode::Full).await?;
//! while let Some(item) = events.next().await {
//!     match item? {
//!         TickerEvent::Raw(observation) => {
//!             let _bytes = observation.payload().as_bytes();
//!         }
//!         TickerEvent::Lifecycle(event) => {
//!             let _what = event.kind();
//!         }
//!         _ => {}
//!     }
//! }
//! let _outcome = guard.join().await;
//! # Ok(()) }
//! # }
//! ```
//!
//! There is no login flow: obtain the request token yourself, then use the
//! session resource's explicit token exchange and invalidation. The legacy
//! [`WebSocketClient`](kite::ticker::WebSocketClient) remains, deprecated,
//! during the migration.
//!
//! # Disclaimer
//!
//! **Important Notice**:
//!
//! * The `manja` crate is currently in development and should be considered unstable. The API is subject to change without notice, and breaking changes are likely to occur.
//!
//! * The software is provided "as-is" without any warranties, express or implied. The author and contributors of this SDK do not take responsibility for any financial losses, damages, or other issues that may arise from the use of this project.
#![warn(rust_2018_idioms)]

pub mod kite;
