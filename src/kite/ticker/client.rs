//! Asynchronous WebSocket client.
//!
//! This module provides functionality for establishing and managing WebSocket
//! connections to Kite Connect streaming API. It includes the `TickerStream`
//! struct for handling the WebSocket stream and the `WebSocketClient` struct
//! for managing the connection and interaction with the WebSocket.
//!
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

/// Represents a WebSocket stream to Kite Connect streaming API.
///
/// This struct holds the WebSocket stream and its state, allowing for interaction
/// with the KiteConnect ticker API.
///
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
            info!(uri = %kite_uri, "ticker.connect.start");
            match tokio_tungstenite::connect_async(kite_uri).await {
                Ok((mut ws_stream, response)) => {
                    info!("ticker.connect.success (status: {})", response.status());
                    info!("Response contains the following headers:");
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
                        stream_state: stream_state,
                    })
                }
                Err(e) => {
                    error!("ticker.connect.error: {}", e);
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("Big problem := {}", e),
                    ))
                }
            }
        })
    }
}

/// Represents a WebSocket client for Kite Connect streaming API.
///
/// This struct manages the WebSocket connection and provides methods to
/// interact with the WebSocket stream.
///
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
    /// ```ignore
    /// use futures_util::stream::StreamExt;
    ///
    /// let stream_state = StreamState::from_credentials(stream_creds)
    ///     .subscribe_token(Mode::Full, 408065)    // INFY
    ///     .subscribe_token(Mode::Full, 884737);   // TATAMOTORS
    /// if let Ok(mut ticker) = WebSocketClient::connect(stream_state).await {
    ///     if let Some(maybe_msg) = ticker.next().await {
    ///         match maybe_msg {
    ///             Ok(msg) => info!("Message: {}", msg),
    ///             Err(e) => error!("Error: {}", e),
    ///         }
    ///     }
    /// }
    /// ```
    ///
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
