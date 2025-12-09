//! Async WebSocket client and additional functionality.
//!
//! This module provides an easy-to-use interface for connecting to the Kite Connect
//! WebSocket API, managing subscriptions to instrument tokens, and receiving real-time
//! streaming market data in various modes such as `Full`, `Quote`, and `LTP` (Last
//! Traded Price).
//!
//! # Features
//!
//! - **WebSocket Client**: Establishes and maintains a reliable, stateful WebSocket
//!     connection to Kite Connect API.
//! - **Subscription Management**: Allows subscribing and unsubscribing to specific
//!     instrument tokens and managing the mode of data reception.
//! - **Real-Time Data Streaming**: Stream market data such as price updates, order
//!     book changes, and more in real-time through the WebSocket connection.
//!
//! # Example Usage
//!
//! ```ignore
//! use kite::ticker::{KiteTickerClient, Mode, TickerRequest};
//! use futures_util::stream::StreamExt;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Assuming we have a request token
//!     let kite_session = manja_client
//!         .session()
//!         .generate_session(&request_token)
//!         .await?;
//!     let stream_creds = KiteStreamCredentials::from(kite_session.data.unwrap());
//!     let stream_state = StreamState::from_credentials(stream_creds)
//!         .subscribe_token(Mode::Full, 408065)    // INFY
//!         .subscribe_token(Mode::Full, 884737);   // TATAMOTORS
//!      
//!     if let Ok(mut ticker) = WebSocketClient::connect(stream_state).await {     
//!         if let Some(maybe_msg) = ticker.next().await {
//!             match maybe_msg {
//!                 Ok(msg) => info!("Message: {}", msg),
//!                 Err(e) => error!("Error: {}", e),
//!             }
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```
//!

// Contains the `WebSocketClient` and `TickerStream` structs, which are used to
// connect to the WebSocket API and handle data streaming.
mod client;
#[allow(unused_imports)]
pub use client::{TickerStream, WebSocketClient};

// Defines the structures for managing the stream state and credentials, including
// `KiteStreamCredentials` and `StreamState`.
mod stream;
#[allow(unused_imports)]
pub use stream::{KiteStreamCredentials, StreamState};

// Contains data models and request types like `Mode` and `TickerRequest` used
// for interacting with the WebSocket API.
mod models;
#[allow(unused_imports)]
pub use models::{Mode, TickerRequest};
