use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use manja_ticker::{Mode, StreamState, WebSocketClient};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::accept_async;
use tungstenite::Message;

/// Helper to bind a local TCP listener on an ephemeral port.
async fn bind_test_listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind test listener");
    let addr = listener.local_addr().expect("failed to read listener addr");
    (listener, addr)
}

/// Build a `StreamState` pointing at the given local WebSocket address.
fn test_stream_state(addr: SocketAddr) -> StreamState {
    let api_base = format!("ws://{}", addr);
    StreamState::from_parts(api_base.as_str(), "TEST_API_KEY", "TEST_ACCESS_TOKEN")
        .subscribe_token(Mode::Full, 408065)
}

/// Basic happy-path: the ticker connects to a local WebSocket server and
/// receives at least one message sent by the server.
///
/// This codifies the connect → subscribe → receive flow described in PH3-T10
/// using an in-process test server rather than live Kite infrastructure.
#[tokio::test]
async fn ticker_receives_message_from_local_server() {
    let (listener, addr) = bind_test_listener().await;

    let server = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("server failed to accept connection");
        let mut ws = accept_async(stream)
            .await
            .expect("server failed to complete websocket handshake");

        // Consume the initial subscription message from the client (one frame
        // because we subscribe a single mode in `test_stream_state`).
        let _subscription = ws
            .next()
            .await
            .expect("expected subscription frame from client")
            .expect("subscription frame decode error");

        // Send a single test payload and then close cleanly.
        ws.send(Message::Text("test-payload".to_owned()))
            .await
            .expect("server failed to send payload");
        ws.close(None)
            .await
            .expect("server failed to send close frame");
    });

    let stream_state = test_stream_state(addr);
    let mut ticker =
        WebSocketClient::connect(stream_state).await.expect("ticker failed to connect");

    let next = timeout(Duration::from_secs(2), ticker.next())
        .await
        .expect("ticker.next timed out before receiving message");

    match next {
        Some(Ok(Message::Text(text))) => {
            assert_eq!(text, "test-payload");
        }
        Some(Ok(other)) => panic!("expected text message from server, got {:?}", other),
        Some(Err(e)) => panic!("unexpected ticker error on first message: {}", e),
        None => panic!("ticker stream ended before delivering first message"),
    }

    // Ensure the server task shuts down promptly once the client has
    // processed the message and close frame.
    timeout(Duration::from_secs(1), server)
        .await
        .expect("server task did not complete in time")
        .expect("server task panicked");
}

/// When the server closes the WebSocket connection after sending a frame, the
/// ticker must make progress: it should either surface a terminal error or
/// end the stream, but it must not hang indefinitely.
///
/// As of PH3-T10, this test documents the observed behavior (error vs. end of
/// stream) as the explicit contract until reconnect policies are refined.
#[tokio::test]
async fn ticker_makes_progress_after_server_disconnect() {
    let (listener, addr) = bind_test_listener().await;

    let server = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("server failed to accept connection");
        let mut ws = accept_async(stream)
            .await
            .expect("server failed to complete websocket handshake");

        // Initial subscription frame from the client.
        let _subscription = ws
            .next()
            .await
            .expect("expected subscription frame from client")
            .expect("subscription frame decode error");

        // Deliver a single message and then close the connection from the
        // server side.
        ws.send(Message::Text("one-shot".to_owned()))
            .await
            .expect("server failed to send payload");
        ws.close(None)
            .await
            .expect("server failed to send close frame");
    });

    let stream_state = test_stream_state(addr);
    let mut ticker =
        WebSocketClient::connect(stream_state).await.expect("ticker failed to connect");

    // First message: should be the server payload.
    let first = timeout(Duration::from_secs(2), ticker.next())
        .await
        .expect("ticker.next timed out before first message");
    match first {
        Some(Ok(Message::Text(text))) => assert_eq!(text, "one-shot"),
        other => panic!("unexpected first ticker item after connect: {:?}", other),
    }

    // After the server-initiated close, the ticker must not stall forever.
    // We accept either a clean end-of-stream or a surfaced error as the
    // current contract; both represent bounded behavior without hangs.
    let second = timeout(Duration::from_secs(2), ticker.next())
        .await
        .expect("ticker.next timed out after server disconnect");

    match second {
        None => {
            // Stream ended cleanly after server close.
        }
        Some(Err(_)) => {
            // Error surfaced after server close; also acceptable as an
            // explicit contract until reconnect semantics are refined.
        }
        Some(Ok(Message::Close(_))) => {
            // A close frame is also a valid, bounded outcome after a
            // server-initiated shutdown.
        }
        Some(Ok(msg)) => panic!(
            "expected ticker to end, error, or yield a close frame after server close; \
             got additional item: {:?}",
            msg
        ),
    }

    timeout(Duration::from_secs(1), server)
        .await
        .expect("server task did not complete in time")
        .expect("server task panicked");
}

/// Cancellation and shutdown: when a consumer task is cancelled, the ticker
/// stream must allow the task to terminate without hanging, and the test
/// harness should observe no runaway work (best-effort, via timeouts).
#[tokio::test]
async fn ticker_consumer_task_terminates_on_cancellation() {
    use tokio::sync::oneshot;

    let (listener, addr) = bind_test_listener().await;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("server failed to accept connection");
        let mut ws = accept_async(stream)
            .await
            .expect("server failed to complete websocket handshake");

        // Consume the subscription frame, then keep the connection open until
        // the test signals shutdown.
        let _subscription = ws
            .next()
            .await
            .expect("expected subscription frame from client")
            .expect("subscription frame decode error");

        // Wait for shutdown signal; this models a long-lived ticker
        // connection without sending additional frames.
        let _ = shutdown_rx.await;
        ws.close(None)
            .await
            .expect("server failed to send close frame on shutdown");
    });

    let stream_state = test_stream_state(addr);
    let ticker =
        WebSocketClient::connect(stream_state).await.expect("ticker failed to connect");

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

    // Drive the ticker in a background task that can be cancelled via
    // `cancel_tx`. Dropping the task or signalling cancellation should not
    // cause hangs or panics.
    let consumer = tokio::spawn(async move {
        let mut ticker = ticker;
        tokio::select! {
            _ = cancel_rx => {
                // Cancellation requested: stop consuming.
            }
            _ = async {
                while let Some(_item) = ticker.next().await {}
            } => {}
        }
    });

    // Give the consumer a bounded window to start polling before cancelling.
    // We avoid explicit sleeps and instead rely on the fact that the `connect`
    // call above only returns once the initial handshake has completed.
    cancel_tx
        .send(())
        .expect("failed to send cancellation signal to consumer task");

    timeout(Duration::from_secs(2), consumer)
        .await
        .expect("consumer task did not terminate after cancellation")
        .expect("consumer task panicked");

    // Signal the server to shut down and ensure it terminates promptly too.
    shutdown_tx
        .send(())
        .expect("failed to send shutdown signal to server");

    timeout(Duration::from_secs(2), server)
        .await
        .expect("server task did not complete after shutdown")
        .expect("server task panicked");
}
