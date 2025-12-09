//! Credentials and stream state management capabilities.
//!
//! This module provides the functionality required to manage WebSocket streams
//! for interacting with the Kite Connect API. It includes structures and methods
//! to handle stream state, credentials, and subscriptions to instrument tokens
//! for various modes.
//!
//! The module is designed to facilitate real-time data streaming from the Kite
//! Connect WebSocket API. It allows clients to subscribe to instrument tokens
//! in different modes (such as Full, Quote, and LTP), manage their subscriptions,
//! and handle the streaming of data efficiently.
//!
//! The module also provides utility functions and implementations to facilitate
//! the connection to the WebSocket API and the handling of subscriptions.
//!
use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::kite::connect::models::UserSession;
use crate::kite::error::Result;
use crate::kite::ticker::models::Mode;

use futures_util::Stream;
use secrecy::{ExposeSecret, Secret};
use tungstenite::{client::IntoClientRequest, Message};

use super::models::TickerRequest;

/// Default WebSocket API base url
///
pub const KITECONNECT_WSS_API_BASE: &str = "wss://ws.kite.trade";

/// Represents the credentials required to authenticate with Kite Connect WebSocket API.
///
#[derive(Debug, Clone)]
pub struct KiteStreamCredentials {
    api_key: Secret<String>,
    access_token: Secret<String>,
}

impl KiteStreamCredentials {
    /// Creates a new `KiteStreamCredentials` instance from the provided API key
    /// and access token.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The API key for KiteConnect.
    /// * `access_token` - The access token for KiteConnect.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let credentials = KiteStreamCredentials::from_parts("api_key", "access_token");
    /// ```
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
///
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
    ///
    /// # Arguments
    ///
    /// * `api_base` - The base URL for Kite Connect WebSocket API.
    /// * `api_key` - The API key for Kite Connect.
    /// * `access_token` - The access token for Kite Connect.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let stream_state = StreamState::from_parts("wss://ws.kite.trade", "api_key", "access_token");
    /// ```
    ///
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
    ///
    /// # Arguments
    ///
    /// * `credentials` - The `KiteStreamCredentials` instance.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let credentials = KiteStreamCredentials::from_parts("api_key", "access_token");
    /// let stream_state = StreamState::from_credentials(credentials);
    /// ```
    ///
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
    ///
    /// # Returns
    ///
    /// A `SubscriptionStream` instance.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let subscription_stream = stream_state.to_subscription_stream();
    /// ```
    ///
    pub fn to_subcription_stream(self) -> SubscriptionStream {
        SubscriptionStream::with_subscription(&self.subscription)
    }

    /// Subscribes to an instrument token with a specified mode.
    ///
    /// # Arguments
    ///
    /// * `mode` - The mode for the subscription.
    /// * `token` - The instrument token to subscribe to.
    ///
    /// # Returns
    ///
    /// The updated `StreamState` instance.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let updated_state = stream_state.subscribe_token(Mode::Full, 12345);
    /// ```
    ///
    pub fn subscribe_token(mut self, mode: Mode, token: u32) -> Self {
        if let Some(vec) = self.subscription.get_mut(&mode) {
            vec.push(token);
        } else {
            self.subscription.insert(mode, vec![token]);
        }
        self
    }

    /// Converts the stream state to a URI string.
    ///
    /// # Returns
    ///
    /// A `String` representing the URI.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let uri = stream_state.to_uri();
    /// ```
    ///
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
///
/// # Role of `current_key_idx` field
///
/// The `current_key_idx` field in the `SubscriptionStream` struct is used to keep
/// track of the current position within the keys of the subscription HashMap.
/// This field is crucial for iterating over the keys in the subscription HashMap
/// and ensures that all modes and their corresponding instrument tokens are processed
/// sequentially.
///
/// 1. *Initialization*: When a `SubscriptionStream` is created, `current_key_idx`
///     is initialized to 0. This means the stream will start processing from the
///     first key in the keys vector.
/// 2. *Iteration*: The `poll_next` method uses `current_key_idx` to determine the
///     current `Mode` being processed.
/// 3. *Processing*: If there are tokens associated with the current mode, a `TickerRequest`
///     is created and serialized to JSON. The `current_key_idx` is then incremented
///     to move to the next mode for the next poll.
/// 4. *Completion*: If `current_key_idx` exceeds the length of the `keys` vector,
///     it means all keys have been processed, and the stream signals completion
///     by returning `Poll::Ready(None)`.
/// 5. *Pending*: If there are no tokens for the current mode, `current_key_idx`
///     is incremented, and the method signals that it is still pending by returning
///     `Poll::Pending`.
///
/// The `current_key_idx` field ensures that each mode and its corresponding tokens
/// are processed in order, and it keeps track of the current position within the
/// iteration. This allows the `poll_next` method to efficiently handle the subscription
/// stream by processing each mode sequentially and appropriately signaling when
/// the stream is complete or pending.
///
pub struct SubscriptionStream {
    /// A mapping of modes to their respective instrument tokens.
    ///
    /// This `HashMap` contains the subscription data where each `Mode` is associated
    /// with a vector of `InstrumentToken`s.
    ///
    pub data: Subscription,

    /// A vector of the keys (modes) from the `subscription` data.
    ///
    /// This vector contains all the modes present in the subscription data and
    /// is used for iterating over the subscription entries, and is generated when
    /// the `SubscriptionStream` is created.
    ///
    pub keys: Vec<Mode>,

    /// An index tracking the current position within the `keys` vector.
    ///
    /// This index is used to keep track of the current mode being processed in
    /// the subscription stream. It starts at 0 and is incremented as each mode
    /// is processed.
    ///
    pub current_key_idx: usize,
}

impl SubscriptionStream {
    /// Creates a new `SubscriptionStream` instance from the provided subscription.
    ///
    /// # Arguments
    ///
    /// * `subscription` - A reference to the subscription data.
    ///
    /// # Returns
    ///
    /// A new `SubscriptionStream` instance.
    ///
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
            keys: keys,
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
    //
    // The method works by iterating over the `keys` (modes) stored in the
    // `SubscriptionStream`. For each mode, it retrieves the associated instrument
    // tokens, creates a `TickerRequest` for the mode and tokens, and serializes
    // the request to a JSON message.
    //
    // If the end of the `keys` list is reached, the stream is considered complete.
    // If a mode has no associated tokens, the method moves to the next mode and continues polling.
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Check if all modes have been processed
        if this.current_key_idx >= this.keys.len() {
            // No more items to stream, return `Poll::Ready(None)` to indicate completion
            return Poll::Ready(None);
        }

        // Get the current mode using the index
        let current_key = &this.keys[this.current_key_idx];
        if let Some(tokens) = this.data.get(current_key) {
            // Move to the next mode for the next poll
            this.current_key_idx += 1;

            // Create a `TickerRequest` for the current mode and tokens
            let ticker_request =
                TickerRequest::subscribe_with_mode(tokens.clone(), current_key.clone());

            // Serialize the `TickerRequest` to JSON and wrap it in a `Message::Text`
            match serde_json::to_string(&ticker_request) {
                Ok(json) => Poll::Ready(Some(Ok(Message::Text(json)))),
                Err(e) => Poll::Ready(Some(Err(e.into()))),
            }
        } else {
            // If no tokens are associated with the current mode, move to the next mode
            this.current_key_idx += 1;
            // Wake the task to continue polling
            cx.waker().wake_by_ref();
            // Indicate that polling is pending and needs to be retried
            Poll::Pending
        }
    }
}
