//! Loopback HTTP/1.1 harness with request capture and scripted fault injection.
//!
//! Each harness binds its own `127.0.0.1` port, so tests configure clients
//! per test through [`HttpHarness::base_url`] rather than through process
//! environment. Every accepted connection consumes the next scripted
//! [`Reply`] and carries exactly one request; normal responses close the
//! connection. A connection with no reply left is closed without a response.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

/// What the harness does with one connection.
pub enum Reply {
    /// A complete response. `body` is sent byte-for-byte as given, so an
    /// official fixture is served unchanged and a malformed body stays malformed.
    Respond {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
    },
    /// Response loss: read the request, then close without responding.
    DropAfterRequest,
    /// Send failure: close as soon as the connection is accepted, before
    /// reading anything.
    CloseOnAccept,
    /// Close/EOF mid-response: announce the whole `body` in `Content-Length`,
    /// write only its first `sent` bytes, then close.
    TruncateBody {
        status: u16,
        body: Vec<u8>,
        sent: usize,
    },
    /// Stalled peer: read the request, then hold the connection open silently
    /// until the harness is dropped.
    Stall,
}

impl Reply {
    /// A `200 OK` JSON response with `body` unchanged.
    pub fn json(body: impl Into<Vec<u8>>) -> Self {
        Self::Respond {
            status: 200,
            content_type: "application/json",
            body: body.into(),
        }
    }
}

/// One request as received by the harness.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    /// Request target: path plus any query string.
    pub target: String,
    /// Header names are lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    /// First value of header `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }
}

pub struct HttpHarness {
    addr: std::net::SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    task: JoinHandle<()>,
}

impl HttpHarness {
    /// Start serving `replies`, one per accepted connection, in order.
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            // Owning the per-connection tasks here aborts them, stalled ones
            // included, when the harness aborts this task on drop.
            let mut connections = JoinSet::new();
            let mut replies = replies.into_iter();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let reply = replies.next();
                connections.spawn(serve(stream, reply, recorded.clone()));
            }
        });
        Self {
            addr,
            requests,
            task,
        }
    }

    /// `http://127.0.0.1:<port>`, with no trailing slash.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Requests received so far, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for HttpHarness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A loopback base URL on which nothing is listening: connecting fails.
pub async fn refused_base_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

async fn serve(
    mut stream: TcpStream,
    reply: Option<Reply>,
    recorded: Arc<Mutex<Vec<RecordedRequest>>>,
) {
    if matches!(reply, Some(Reply::CloseOnAccept)) {
        return;
    }
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    recorded.lock().unwrap().push(request);
    match reply {
        None | Some(Reply::CloseOnAccept) | Some(Reply::DropAfterRequest) => {}
        Some(Reply::Respond {
            status,
            content_type,
            body,
        }) => {
            let head = response_head(status, content_type, body.len());
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        }
        Some(Reply::TruncateBody { status, body, sent }) => {
            let head = response_head(status, "application/json", body.len());
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&body[..sent.min(body.len())]).await;
        }
        Some(Reply::Stall) => {
            std::future::pending::<()>().await;
        }
    }
    let _ = stream.shutdown().await;
}

fn response_head(status: u16, content_type: &str, len: usize) -> String {
    format!(
        "HTTP/1.1 {status} Harness\r\ncontent-type: {content_type}\r\n\
         content-length: {len}\r\nconnection: close\r\n\r\n"
    )
}

/// Read one request head and a `Content-Length` body. Chunked request bodies
/// are not supported; `None` means the peer closed before a full head arrived.
async fn read_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    let mut buf = Vec::new();
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_string();
    let target = request_line.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(RecordedRequest {
        method,
        target,
        headers,
        body,
    })
}
