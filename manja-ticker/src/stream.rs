//! Credentials and stream state management capabilities for the ticker.
//!
//! This module is extracted from the `manja` facade crate and is responsible
//! for managing WebSocket stream state, credentials, and subscription streams.

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use manja_core::models::UserSession;
use secrecy::{ExposeSecret, Secret};
use tracing::debug;
use tungstenite::{client::IntoClientRequest, Message};

use crate::models::{Mode, TickerRequest};

type Result<T> = std::result::Result<T, serde_json::Error>;

/// Default WebSocket API base url.
pub const KITECONNECT_WSS_API_BASE: &str = "wss://ws.kite.trade";

/// Represents the credentials required to authenticate with Kite Connect WebSocket API.
#[derive(Debug, Clone)]
pub struct KiteStreamCredentials {
    api_key: Secret<String>,
    access_token: Secret<String>,
}

impl KiteStreamCredentials {
    /// Creates a new `KiteStreamCredentials` instance from the provided API key
    /// and access token.
    pub fn from_parts<InS>(api_key: InS, access_token: InS) -> Self
    where
        InS: Into<String>,
    {
        Self {
            api_key: Secret::new(api_key.into()),
            access_token: Secret::new(access_token.into()),
        }
    }

    // Converts the credentials into a query parameter string.
    fn to_query_params(&self) -> String {
        format!(
            "api_key={}&access_token={}",
            self.api_key.expose_secret(),
            self.access_token.expose_secret()
        )
    }
}

impl From<UserSession> for KiteStreamCredentials {
    fn from(value: UserSession) -> Self {
        Self {
            api_key: value.api_key,
            access_token: value.access_token,
        }
    }
}

type InstrumentToken = u32;

/// A mapping of subscribed instruments on a WebSocket connection.
///
/// This map stores the instrument tokens that are actively subscribed to via
/// a WebSocket connection, allowing for real-time streaming of market data
/// for those instruments.
type Subscription = HashMap<Mode, Vec<InstrumentToken>>;

/// Represents the state of the WebSocket stream (connection).
#[derive(Debug, Clone)]
pub struct StreamState {
    // The base URL for Kite Connect WebSocket API.
    api_base: String,
    // Credentials for accessing Kite Connect WebSocket API: `api_key` and `access_token`.
    credentials: KiteStreamCredentials,
    // Subscribed instruments on a WebSocket stream (connection).
    subscription: Subscription,
}

impl StreamState {
    /// Creates a new `StreamState` instance from the provided API base URL,
    /// API key, and access token.
    pub fn from_parts<InS>(api_base: InS, api_key: InS, access_token: InS) -> Self
    where
        InS: Into<String>,
    {
        Self {
            api_base: api_base.into(),
            credentials: KiteStreamCredentials::from_parts(api_key, access_token),
            subscription: Default::default(),
        }
    }

    /// Creates a new `StreamState` instance from the provided credentials.
    pub fn from_credentials(credentials: KiteStreamCredentials) -> Self {
        let api_base = std::env::var("KITECONNECT_WSS_API_BASE")
            .unwrap_or_else(|_| KITECONNECT_WSS_API_BASE.to_string())
            .into();
        Self {
            api_base,
            credentials,
            subscription: Default::default(),
        }
    }

    /// Converts the stream state to a subscription stream.
    pub fn to_subcription_stream(self) -> SubscriptionStream {
        SubscriptionStream::with_subscription(&self.subscription)
    }

    /// Subscribes to an instrument token with a specified mode.
    pub fn subscribe_token(mut self, mode: Mode, token: u32) -> Self {
        let mode_key = mode.clone();
        if let Some(vec) = self.subscription.get_mut(&mode_key) {
            vec.push(token);
        } else {
            self.subscription.insert(mode_key.clone(), vec![token]);
        }
        if let Some(tokens) = self.subscription.get(&mode_key) {
            debug!(
                mode = ?mode_key,
                token,
                token_count = tokens.len(),
                "ticker.subscription.update"
            );
        }
        self
    }

    /// Converts the stream state to a URI string.
    pub fn to_uri(&self) -> String {
        format!("{}?{}", self.api_base, self.credentials.to_query_params())
    }
}

impl IntoClientRequest for StreamState {
    fn into_client_request(self) -> tungstenite::Result<tungstenite::handshake::client::Request> {
        format!("{}?{}", self.api_base, self.credentials.to_query_params()).into_client_request()
    }
}

/// Represents a stream of subscriptions to instrument tokens for different modes.
///
/// The `SubscriptionStream` struct handles iterating over the subscribed modes
/// and their corresponding instrument tokens, generating WebSocket messages for
/// each subscription.
pub struct SubscriptionStream {
    /// A mapping of modes to their respective instrument tokens.
    pub data: Subscription,
    /// A vector of the keys (modes) from the `subscription` data.
    pub keys: Vec<Mode>,
    /// An index tracking the current position within the `keys` vector.
    pub current_key_idx: usize,
}

impl SubscriptionStream {
    /// Creates a new `SubscriptionStream` instance from the provided subscription.
    pub fn with_subscription(subcription: &Subscription) -> Self {
        Self {
            data: subcription.clone(),
            keys: subcription.keys().cloned().collect(),
            current_key_idx: 0,
        }
    }
}

impl From<StreamState> for SubscriptionStream {
    fn from(value: StreamState) -> Self {
        let keys = value.subscription.keys().cloned().collect();
        Self {
            data: value.subscription,
            keys,
            current_key_idx: 0,
        }
    }
}

impl Stream for SubscriptionStream {
    type Item = Result<Message>;

    // Polls the next item in the subscription stream.
    //
    // This function polls the subscription stream for the next message,
    // returning it as a `Poll` wrapped `Result`.
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Check if all modes have been processed.
        if this.current_key_idx >= this.keys.len() {
            return Poll::Ready(None);
        }

        // Get the current mode using the index.
        let current_key = &this.keys[this.current_key_idx];
        if let Some(tokens) = this.data.get(current_key) {
            // Move to the next mode for the next poll.
            this.current_key_idx += 1;

            // Create a `TickerRequest` for the current mode and tokens.
            let ticker_request =
                TickerRequest::subscribe_with_mode(tokens.clone(), current_key.clone());

            debug!(mode = ?current_key, token_count = tokens.len(), "ticker.subscription.request");

            // Serialize the `TickerRequest` to JSON and wrap it in a `Message::Text`.
            match serde_json::to_string(&ticker_request) {
                Ok(json) => Poll::Ready(Some(Ok(Message::Text(json)))),
                Err(e) => Poll::Ready(Some(Err(e))),
            }
        } else {
            // If no tokens are associated with the current mode, move to the next mode.
            this.current_key_idx += 1;
            // Wake the task to continue polling.
            cx.waker().wake_by_ref();
            // Indicate that polling is pending and needs to be retried.
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_state_to_uri_from_parts() {
        let state = StreamState::from_parts(
            "wss://example.test/ticker",
            "TEST_API_KEY",
            "TEST_ACCESS_TOKEN",
        );
        let uri = state.to_uri();
        assert!(
            uri == "wss://example.test/ticker?api_key=TEST_API_KEY&access_token=TEST_ACCESS_TOKEN"
                || uri == "wss://example.test/ticker?access_token=TEST_ACCESS_TOKEN&api_key=TEST_API_KEY"
        );
    }

    #[test]
    fn stream_state_to_uri_from_credentials_uses_env_api_base() {
        std::env::set_var("KITECONNECT_WSS_API_BASE", "wss://example-env.test");
        let creds = KiteStreamCredentials::from_parts("ENV_API_KEY", "ENV_ACCESS_TOKEN");
        let state = StreamState::from_credentials(creds);
        let uri = state.to_uri();
        assert!(
            uri == "wss://example-env.test?api_key=ENV_API_KEY&access_token=ENV_ACCESS_TOKEN"
                || uri == "wss://example-env.test?access_token=ENV_ACCESS_TOKEN&api_key=ENV_API_KEY"
        );
        // Clean up for other tests.
        std::env::remove_var("KITECONNECT_WSS_API_BASE");
    }

    #[test]
    fn subscription_stream_emits_ticker_requests() {
        let state = StreamState::from_parts(
            "wss://example.test/ticker",
            "TEST_API_KEY",
            "TEST_ACCESS_TOKEN",
        )
        .subscribe_token(Mode::Full, 408065)
        .subscribe_token(Mode::Quote, 884737);

        let mut stream = SubscriptionStream::from(state);

        let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
        let mut pinned = Pin::new(&mut stream);

        let first = pinned.as_mut().poll_next(&mut cx);
        let second = pinned.as_mut().poll_next(&mut cx);

        let first_msg = match first {
            Poll::Ready(Some(Ok(Message::Text(json)))) => json,
            other => panic!("unexpected first poll result: {:?}", other),
        };
        let second_msg = match second {
            Poll::Ready(Some(Ok(Message::Text(json)))) => json,
            other => panic!("unexpected second poll result: {:?}", other),
        };

        // Ensure the payloads are valid JSON ticker requests.
        serde_json::from_str::<serde_json::Value>(&first_msg).unwrap();
        serde_json::from_str::<serde_json::Value>(&second_msg).unwrap();
    }
}
