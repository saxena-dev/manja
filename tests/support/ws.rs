//! Loopback WebSocket harness with handshake capture and scripted faults.
//!
//! Each harness binds its own `127.0.0.1` port; clients are pointed at
//! [`WsHarness::url`] per test. Every accepted TCP connection consumes the next
//! scripted [`WsConnection`]; a connection with no script left is closed before
//! the handshake. Messages the client sends are recorded for inspection.

use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http;
use tokio_tungstenite::tungstenite::Message;

/// How the harness answers the opening handshake.
pub enum Handshake {
    Accept,
    /// Handshake failure: answer the upgrade request with this HTTP status.
    Reject(u16),
}

/// One scripted action after a successful handshake.
pub enum Step {
    /// Send this message unchanged. Arbitrary text or binary payloads model
    /// malformed or truncated packets.
    Send(Message),
    /// Send a close frame, then stop.
    Close,
    /// Close/EOF: drop the TCP connection without a close frame. A client that
    /// keeps sending afterwards observes a send failure.
    Eof,
}

/// The script for one connection. If `steps` does not end in [`Step::Close`] or
/// [`Step::Eof`], the harness becomes a stalled peer: it sends nothing more but
/// keeps the connection open, still recording client messages, until dropped.
pub struct WsConnection {
    pub handshake: Handshake,
    pub steps: Vec<Step>,
}

/// The opening handshake request as received.
#[derive(Clone, Debug)]
pub struct RecordedHandshake {
    /// Request target: path plus query string.
    pub target: String,
    /// Header names are lower-cased.
    pub headers: Vec<(String, String)>,
}

#[derive(Default)]
struct Recorded {
    handshakes: Vec<RecordedHandshake>,
    messages: Vec<Message>,
}

pub struct WsHarness {
    addr: std::net::SocketAddr,
    recorded: Arc<Mutex<Recorded>>,
    task: JoinHandle<()>,
}

impl WsHarness {
    /// Start serving `connections`, one per accepted TCP connection, in order.
    pub async fn start(connections: Vec<WsConnection>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let shared = recorded.clone();
        let task = tokio::spawn(async move {
            let mut served = JoinSet::new();
            let mut connections = connections.into_iter();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                if let Some(connection) = connections.next() {
                    served.spawn(serve(stream, connection, shared.clone()));
                }
            }
        });
        Self {
            addr,
            recorded,
            task,
        }
    }

    /// `ws://127.0.0.1:<port>`, with no trailing slash.
    pub fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    /// Opening handshakes received so far, in arrival order.
    pub fn handshakes(&self) -> Vec<RecordedHandshake> {
        self.recorded.lock().unwrap().handshakes.clone()
    }

    /// Messages received from clients so far, in arrival order.
    pub fn messages(&self) -> Vec<Message> {
        self.recorded.lock().unwrap().messages.clone()
    }
}

impl Drop for WsHarness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Aborts the wrapped task when dropped, so a connection's reader never
/// outlives the connection's own task.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn serve(stream: TcpStream, connection: WsConnection, recorded: Arc<Mutex<Recorded>>) {
    let reject = match connection.handshake {
        Handshake::Accept => None,
        Handshake::Reject(status) => Some(status),
    };
    // The `Result<Response, ErrorResponse>` shape is fixed by tungstenite's
    // handshake `Callback` trait, so the large error type cannot be boxed.
    #[allow(clippy::result_large_err)]
    let on_handshake = {
        let recorded = recorded.clone();
        move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            let headers = request
                .headers()
                .iter()
                .map(|(k, v)| {
                    let value = String::from_utf8_lossy(v.as_bytes()).into_owned();
                    (k.as_str().to_ascii_lowercase(), value)
                })
                .collect();
            recorded.lock().unwrap().handshakes.push(RecordedHandshake {
                target: request.uri().to_string(),
                headers,
            });
            match reject {
                None => Ok(response),
                Some(status) => Err(http::Response::builder().status(status).body(None).unwrap()),
            }
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, on_handshake).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    let reader = AbortOnDrop(tokio::spawn(async move {
        while let Some(Ok(message)) = source.next().await {
            recorded.lock().unwrap().messages.push(message);
        }
    }));
    for step in connection.steps {
        match step {
            Step::Send(message) => {
                if sink.send(message).await.is_err() {
                    return;
                }
            }
            Step::Close => {
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
            Step::Eof => {
                // Dropping both halves drops the socket with no close frame.
                drop(reader);
                drop(sink);
                return;
            }
        }
    }
    let _keep_open = (sink, reader);
    std::future::pending::<()>().await;
}
