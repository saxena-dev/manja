//! Asynchronous WebSocket client for the Kite Connect ticker stream.
//!
//! This module is extracted from the `manja` facade crate and provides
//! the underlying WebSocket runtime used by `manja::kite::ticker`.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::Poll;

use crate::stream::{StreamState, SubscriptionStream};

use futures_util::{SinkExt, Stream, StreamExt};
use stubborn_io::tokio::{StubbornIo, UnderlyingIo};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, span, Level};
use tungstenite::client::IntoClientRequest;

/// Represents a WebSocket stream to Kite Connect streaming API.
///
/// This struct holds the WebSocket stream and its state, allowing for interaction
/// with the KiteConnect ticker API.
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
    fn establish(
        stream_state: StreamState,
    ) -> Pin<Box<dyn Future<Output = io::Result<Self>> + Send>> {
        Box::pin(async move {
            // TODO: Fix `unwrap`
            let request = stream_state.clone().into_client_request().unwrap();
            let uri = request.uri().clone();
            let scheme = uri.scheme_str().unwrap_or("wss").to_string();
            let host = uri.host().unwrap_or_default().to_string();
            let path = uri.path().to_string();

            let span = span!(
                Level::INFO,
                "ticker.connect",
                %scheme,
                %host,
                %path
            );
            let _enter = span.enter();

            info!("ticker.connect.start");

            let connect_uri = uri.to_string();
            match tokio_tungstenite::connect_async(connect_uri).await {
                Ok((mut ws_stream, response)) => {
                    info!(status = %response.status(), "ticker.connect.success");
                    let mut subscribe_stream = SubscriptionStream::from(stream_state.clone());
                    while let Some(maybe_msg) = subscribe_stream.next().await {
                        match maybe_msg {
                            Ok(msg) => {
                                debug!(payload = %msg, "ticker.subscription.send");
                                if let Err(e) = ws_stream.send(msg).await {
                                    error!(error = %e, "ticker.subscription.send_error");
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "ticker.subscription.serialize_error");
                            }
                        }
                    }
                    Ok(TickerStream {
                        ws_stream,
                        stream_state,
                    })
                }
                Err(e) => {
                    error!(error = %e, "ticker.connect.error");
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
pub struct WebSocketClient(StubbornIo<TickerStream, StreamState>);

impl WebSocketClient {
    /// Connects to the WebSocket stream with the given stream state.
    ///
    /// This function establishes a persistent WebSocket connection using the
    /// given stream state.
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
