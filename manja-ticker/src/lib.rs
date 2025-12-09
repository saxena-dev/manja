//! WebSocket ticker client for the `manja` SDK.
//!
//! This crate hosts the async WebSocket ticker runtime used by the
//! `manja` facade crate when the `websocket` feature is enabled.
//!
//! Most users should depend on the `manja` crate and use
//! `manja::kite::ticker`, which re-exports the types defined here. The
//! `manja-ticker` crate is provided for advanced users that want to wire
//! the ticker client directly.
//!
//! # Example
//!
//! ```ignore
//! use futures_util::StreamExt;
//! use manja_core::models::UserSession;
//! use manja_ticker::{KiteStreamCredentials, Mode, StreamState, WebSocketClient};
//!
//! # async fn example(kite_session: UserSession) -> Result<(), Box<dyn std::error::Error>> {
//! let stream_creds = KiteStreamCredentials::from(kite_session);
//! let stream_state = StreamState::from_credentials(stream_creds)
//!     .subscribe_token(Mode::Full, 408065) // INFY
//!     .subscribe_token(Mode::Full, 884737); // TATAMOTORS
//!
//! let mut ticker = WebSocketClient::connect(stream_state).await?;
//! if let Some(maybe_msg) = ticker.next().await {
//!     match maybe_msg {
//!         Ok(msg) => println!("Ticker message: {}", msg),
//!         Err(e) => eprintln!("Ticker error: {}", e),
//!     }
//! }
//! # Ok(()) }
//! ```

pub mod client;
pub mod models;
pub mod stream;

pub use crate::client::{TickerStream, WebSocketClient};
pub use crate::models::{Mode, TickerRequest};
pub use crate::stream::{KiteStreamCredentials, StreamState};

