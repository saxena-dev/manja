//! Loopback servers for the examples. Nothing here reaches the network: every
//! server binds `127.0.0.1` on a free port and serves repository files
//! unchanged, the official `kiteconnect-mocks/` responses and the vendored
//! ticker corpus.
#![allow(dead_code)]

use std::path::PathBuf;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A repository file, by path from the crate root.
pub fn repo_file(path: &str) -> Vec<u8> {
    let full = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
}

/// Serve official HTTP mocks: each request whose path (without query)
/// starts with a route's prefix gets that route's file as a `200` JSON body.
/// Returns the base URL.
pub async fn http_mocks(routes: &[(&'static str, &'static str, &'static str)]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes: Vec<(&str, &str, Vec<u8>)> = routes
        .iter()
        .map(|(method, prefix, file)| {
            (
                *method,
                *prefix,
                repo_file(&format!("kiteconnect-mocks/{file}")),
            )
        })
        .collect();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let routes = routes.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0u8; 4096];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&head);
                let mut line = text.lines().next().unwrap_or("").split(' ');
                let (method, target) = (line.next().unwrap_or(""), line.next().unwrap_or(""));
                let path = target.split('?').next().unwrap_or("");
                let body = routes
                    .iter()
                    .find(|(m, p, _)| *m == method && path.starts_with(p))
                    .map(|(_, _, b)| b.clone());
                let (status, body) = match body {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", br#"{"status":"error","message":"no mock","error_type":"GeneralException"}"#.to_vec()),
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            });
        }
    });
    base
}

/// Serve one WebSocket connection that sends `messages`, then reads until
/// the client closes. Returns the `ws://` URL.
#[cfg(feature = "ticker")]
pub async fn ticker_mock(messages: Vec<tokio_tungstenite::tungstenite::Message>) -> String {
    use futures_util::{SinkExt, StreamExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        let Ok(mut ws) = tokio_tungstenite::accept_async(tcp).await else {
            return;
        };
        for m in messages {
            if ws.send(m).await.is_err() {
                return;
            }
        }
        // Reading also answers the client's close.
        while let Some(Ok(message)) = ws.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(t) = message {
                println!("  server received: {t}");
            }
        }
    });
    url
}
