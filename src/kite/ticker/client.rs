//! The legacy WebSocket client. Deprecated.
//!
//! [`WebSocketClient`] connects with a [`StreamState`], sends that state's
//! `mode` requests (not `subscribe` requests; see
//! [`super::stream`]) and yields the `tungstenite` messages it receives,
//! uninterpreted. Its item type is unchanged:
//! `Result<tungstenite::Message, tungstenite::Error>`.
//!
//! It makes no readiness, reconnect or subscription-restoration
//! guarantee. Polling reads the currently established socket directly, so
//! a dropped connection surfaces as an error or the end of the stream; the
//! wrapper's reconnection is not driven by polling, and nothing is
//! re-sent. It is kept, with unchanged behavior, for existing callers
//! during the migration. New code uses the actor ticker
//! ([`crate::kite::ticker::actor`]).
//!
// The deprecated items are defined and used here.
#![allow(deprecated)]
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::Poll;

use crate::kite::ticker::stream::{StreamState, SubscriptionStream};

use futures_util::{SinkExt, Stream, StreamExt};
use stubborn_io::tokio::{StubbornIo, UnderlyingIo};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info};
use tungstenite::client::IntoClientRequest;

/// The legacy client's socket and state. Deprecated.
///
/// Its fields are public: the socket can be read and written from outside,
/// so nothing about it is owned or guaranteed.
#[deprecated(
    since = "0.2.0",
    note = "the legacy ticker makes no readiness, reconnect or restoration guarantee; use `manja::kite::ticker::actor::owner::TickerBuilder`"
)]
pub struct TickerStream {
    /// WebSocket stream
    pub ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    /// Stream state
    pub stream_state: StreamState,
}

impl UnderlyingIo<StreamState> for TickerStream
where
    StreamState: IntoClientRequest + Clone + Send + Unpin + 'static,
    SubscriptionStream: From<StreamState>,
{
    /// Establishes a connection to the WebSocket stream.
    ///
    /// This function takes the stream state, establishes a connection using
    /// WebSocket, and returns a `TickerStream` instance.
    ///
    /// # Arguments
    ///
    /// * `stream_state` - The state of the stream to be established.
    ///
    /// # Returns
    ///
    /// A pinned future that resolves to an `io::Result` containing a `TickerStream` instance.
    ///
    fn establish(
        stream_state: StreamState,
    ) -> Pin<Box<dyn Future<Output = io::Result<Self>> + Send>> {
        Box::pin(async move {
            // TODO: Fix `unwrap`
            let request = stream_state.clone().into_client_request().unwrap();
            let kite_uri = format!("{}", request.uri());
            match tokio_tungstenite::connect_async(kite_uri).await {
                Ok((mut ws_stream, response)) => {
                    info!("Connected to the server");
                    info!("Response HTTP code: {}", response.status());
                    info!("Response contains the following headers:");
                    for (header, value) in response.headers() {
                        info!("* {}: {:?}", header, value);
                    }
                    let mut subscribe_stream = SubscriptionStream::from(stream_state.clone());
                    while let Some(maybe_msg) = subscribe_stream.next().await {
                        match maybe_msg {
                            Ok(msg) => {
                                debug!("Ticker request: {}", msg);
                                match ws_stream.send(msg).await {
                                    Ok(_) => (),
                                    Err(e) => error!("Error sending a ticker request: {}", e),
                                }
                            }
                            Err(e) => {
                                error!("Error serializing TickerRequest: {}", e)
                            }
                        }
                    }
                    Ok(TickerStream {
                        ws_stream,
                        stream_state,
                    })
                }
                Err(e) => Err(io::Error::other(format!("Big problem := {}", e))),
            }
        })
    }
}

/// The legacy WebSocket client. Deprecated; see the module docs.
#[deprecated(
    since = "0.2.0",
    note = "the legacy ticker makes no readiness, reconnect or restoration guarantee; use `manja::kite::ticker::actor::owner::TickerBuilder`"
)]
pub struct WebSocketClient(StubbornIo<TickerStream, StreamState>);

impl WebSocketClient {
    /// Connects to the WebSocket stream with the given stream state.
    ///
    /// This function establishes a persistent WebSocket connection using the
    /// given stream state.
    ///
    /// # Arguments
    ///
    /// * `stream_state` - The state of the stream to be connected.
    ///
    /// # Returns
    ///
    /// An `io::Result` containing a `WebSocketClient` instance.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #![allow(deprecated)]
    /// use futures_util::StreamExt;
    /// use manja::kite::ticker::{Mode, StreamState, WebSocketClient};
    ///
    /// # async fn run() -> std::io::Result<()> {
    /// let state = StreamState::from_parts("wss://ws.kite.trade", "api_key", "access_token")
    ///     .subscribe_token(Mode::Full, 408065);
    /// let mut ticker = WebSocketClient::connect(state).await?;
    /// while let Some(message) = ticker.next().await {
    ///     // Uninterpreted tungstenite messages; nothing is re-sent after a
    ///     // disconnect.
    ///     let _ = message;
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn connect(stream_state: StreamState) -> io::Result<Self> {
        match StubbornIo::connect(stream_state).await {
            Ok(stubborn) => Ok(WebSocketClient(stubborn)),
            Err(e) => Err(e),
        }
    }
}

impl Stream for WebSocketClient {
    type Item = Result<tungstenite::protocol::Message, tungstenite::Error>;

    // Polls the next item in the WebSocket stream.
    //
    // This function polls the WebSocket stream for the next message, returning
    // it as a `Poll` wrapped `Result`.
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0.ws_stream).poll_next(cx)
    }
}
