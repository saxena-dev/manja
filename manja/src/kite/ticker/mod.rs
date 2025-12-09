//! Async WebSocket ticker facade.
//!
//! This module exposes the WebSocket ticker client used for streaming market
//! data from Kite Connect. The underlying implementation lives in the
//! `manja-ticker` crate; this module simply re-exports its primary types so
//! they remain available under `manja::kite::ticker`.
//!
//! The ticker is available when the `websocket` feature is enabled.
//!
//! # Example
//!
//! ```ignore
//! use futures_util::StreamExt;
//! use manja::kite::ticker::{KiteStreamCredentials, Mode, StreamState, WebSocketClient};
//!
//! # async fn example(kite_session: manja::UserSession) -> Result<(), Box<dyn std::error::Error>> {
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

pub use manja_ticker::{
    KiteStreamCredentials, Mode, StreamState, TickerRequest, TickerStream, WebSocketClient,
};
