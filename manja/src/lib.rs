//! > **Manja** (IPA: /maːŋdʒʱaː/) n.: A type of abrasive string utilized primarily for flying fighter kites, especially prevalent in South Asian countries. It is crafted by coating cotton string with powdered glass or a similar abrasive substance.
//!
//! An asynchronous client library for [Zerodha](https://zerodha.com/)'s [Kite Connect](https://kite.trade/)
//! trading APIs (a set of REST-like HTTP APIs).
//!
//! # `manja` Features
//!
//! - **Type safe**
//!    - *Compile-time Type Checking*: type safety ensures that errors related to type mismatches are caught during compilation rather than at runtime.
//!    - *Consistent Data Models*: `manja` uses strongly typed data models that match Kite Connect API's expected inputs and outputs.
//!    - *Enhanced Security*: by ensuring that only valid data types are sent to and received from the API, the risk of data-related vulnerabilities is reduced.
//!    - *Automatic Serialization/Deserialization*: `manja` handles the serialization (converting data structures to JSON) and deserialization (converting JSON responses back to data structures) automatically and correctly. This ensures that the data sent to and received from Kite Connect API adheres to the expected types.
//!    
//! - **Asynchronous**: built on the performant `tokio` async-runtime, `manja` delivers unmatched performance, ensuring your applications run faster and more efficiently than ever before.
//!    - *Resource Efficiency*: maximize the use of your system's resources. `manja`'s asynchronous nature allows for optimal resource management, reducing overhead and improving overall performance.
//!    - *Concurrent Task Handling*: manage multiple tasks simultaneously without sacrificing performance or reliability.
//!    - *Improved latency*: experience reduced latency and faster response times, ensuring your applications are always responsive.
//!
//! - **Distributed Logging**: stay ahead of issues with real-time distributed logging using the `tracing` crate.
//!    - *Streamline Development*: facilitate smoother development cycles with better debugging and faster issue resolution.
//!    - *Reduce Downtime*: with real-time insights and quick access to logs, identify and resolve issues faster, minimizing downtime.
//!    - *Enhance User Experience*: quickly address errors and performance bottlenecks to provide a better experience for your users.
//!
//! - **WebSocket** support for streaming binary market data (via a feature-gated ticker client).
//!    - *Auto-reconnect Mechanism*: `manja` provides a reliable and stateful async WebSocket client with a configurable exponential backoff retry mechanism.
//!
//! - **WebDriver** integration for retrieving `request token` from the redirect URL after successfully authenticating with the Kite platform (when enabled).
//!
//! # Quickstart
//!
//! The recommended entrypoint is the [`ManjaClient`] facade, which wraps the
//! lower-level HTTP client and exposes typed API groups for each Kite domain.
//!
//! ```ignore
//! use manja::ManjaClient;
//! use manja::{KiteApiResponse, UserProfile};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a client using environment-based configuration
//!     let mut client = ManjaClient::from_env();
//!
//!     // Example: log in using a request token obtained via your preferred flow
//!     let request_token = "<request_token>".to_string();
//!     let session = client.session();
//!     let kite_session = session.generate_session(&request_token).await?;
//!
//!     // Example: fetch the user profile
//!     let profile: KiteApiResponse<UserProfile> = client.user().profile().await?;
//!     println!("User: {:?}", profile.data);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Example: Fetching mutual fund orders
//!
//! ```ignore
//! use manja::ManjaClient;
//! use manja::{KiteApiResponse, MfOrder};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut client = ManjaClient::from_env();
//!
//!     // Fetch the latest mutual fund orders (last 7 days).
//!     let response: KiteApiResponse<Vec<MfOrder>> =
//!         client.mutual_funds().orders().await?;
//!
//!     if let Some(orders) = response.data {
//!         for order in orders {
//!             println!("MF order {}: {} {}", order.order_id, order.tradingsymbol, order.status.unwrap_or_default());
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Example: Fetching historical candles
//!
//! The [`ManjaClient::historical`] API group provides typed access to
//! `/instruments/historical/:instrument_token/:interval` and returns
//! [`HistoricalData`] (a sequence of [`HistoricalCandle`] values).
//!
//! ```ignore
//! use chrono::{NaiveDate, NaiveDateTime};
//! use manja::ManjaClient;
//! use manja::{HistoricalData, HistoricalInterval, KiteApiResponse};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a client using environment-based configuration and perform your
//!     // preferred login flow to establish a session (omitted here).
//!     let mut client = ManjaClient::from_env();
//!
//!     // Define the time window in exchange-local time.
//!     let from: NaiveDateTime = NaiveDate::from_ymd_opt(2019, 12, 4)
//!         .unwrap()
//!         .and_hms_opt(9, 15, 0)
//!         .unwrap();
//!     let to: NaiveDateTime = NaiveDate::from_ymd_opt(2019, 12, 4)
//!         .unwrap()
//!         .and_hms_opt(9, 20, 0)
//!         .unwrap();
//!
//!     // Fetch minute candles with OI for a given instrument token.
//!     let response: KiteApiResponse<HistoricalData> = client
//!         .historical()
//!         .candles(
//!             12517890,                      // instrument_token
//!             HistoricalInterval::Minute,    // interval
//!             from,
//!             to,
//!             false,                         // continuous
//!             true,                          // oi
//!         )
//!         .await?;
//!
//!     if let Some(data) = response.data {
//!         for candle in data.candles {
//!             println!(
//!                 "{} O:{:.2} H:{:.2} L:{:.2} C:{:.2} V:{} OI:{:?}",
//!                 candle.timestamp,
//!                 candle.open,
//!                 candle.high,
//!                 candle.low,
//!                 candle.close,
//!                 candle.volume,
//!                 candle.oi,
//!             );
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! For WebSocket streaming or WebDriver-assisted login flows, refer to the
//! module-level documentation under [`kite::ticker`] and [`kite::login`].
//!
//! # Disclaimer
//!
//! **Important Notice**:
//!
//! * The `manja` crate is currently in development and should be considered unstable. The API is subject to change without notice, and breaking changes are likely to occur.
//!
//! * The software is provided "as-is" without any warranties, express or implied. The author and contributors of this SDK do not take responsibility for any financial losses, damages, or other issues that may arise from the use of this project.
#![warn(rust_2018_idioms)]
#![allow(private_interfaces, unused)]

mod client;

pub use client::ManjaClient;

// Core error types and result alias.
pub use crate::kite::error::{KiteApiException, ManjaError, Result};

// Primary HTTP models and enums.
pub use crate::kite::connect::models::{
    Alert, AlertBasket, AlertBasketGttMeta, AlertBasketItem, AlertBasketParams, AlertHistoryEntry,
    AlertHistoryMeta, AlertHistoryOhlc, AlertOperator, AlertRequest, AlertRhsType, AlertStatus,
    AlertType, Auction, Available, BasketMargin, Charges, Exchange, FullQuote, GST, GttCondition,
    GttOrderExecutionResult, GttOrderParams, GttOrderResult, GttStatus, GttTrigger, GttTriggerId,
    GttTriggerRequest, GttType, HistoricalCandle, HistoricalData, HistoricalInterval, Holding,
    Instrument, KiteApiResponse, LTPQuote, MfHolding, MfInstrument, MfOrder, MfSip, OHLCQuote,
    Order, OrderCharges, OrderChargesRequest, OrderMargin, OrderMarginRequest, OrderReceipt,
    OrderStatus, OrderType, OrderValidity, OrderVariety, PNL, Position, PositionConversionRequest,
    ProductType, QuoteMode, Segment, SegmentKind, Trade, TransactionType, UserMargins,
    UserProfile, UserSession, Utilised,
};

pub mod kite;

#[cfg(test)]
pub mod test_support {
    /// Initialize a tracing subscriber for tests when `MANJA_TEST_TRACING` is set.
    ///
    /// This keeps tests quiet by default while allowing opt-in tracing:
    ///
    /// ```bash
    /// MANJA_TEST_TRACING=1 RUST_LOG=trace cargo test my_test -- --nocapture
    /// ```
    pub fn init_tracing() {
        if std::env::var("MANJA_TEST_TRACING").is_err() {
            return;
        }
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .try_init();
    }
}
