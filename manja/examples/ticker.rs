#[cfg(feature = "websocket")]
use futures_util::StreamExt;

#[cfg(feature = "websocket")]
use manja::kite::ticker::{KiteStreamCredentials, Mode, StreamState, WebSocketClient};

#[cfg(feature = "websocket")]
use manja::ManjaClient;

#[cfg(feature = "websocket")]
use tracing::{error, info};

/// Ticker streaming example.
///
/// This example demonstrates how to:
/// - Exchange a request token for an access token using `ManjaClient`.
/// - Construct WebSocket ticker credentials from the resulting session.
/// - Subscribe to a couple of tokens and print incoming messages.
///
/// At runtime this requires valid Kite Connect credentials and a `request_token`
/// (for example via the `KITECONNECT_REQUEST_TOKEN` environment variable).
#[cfg(all(feature = "websocket"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Construct an HTTP+WebSocket client from environment configuration.
    let mut client = ManjaClient::from_env();

    // Obtain the request token used to create a new session.
    let request_token =
        std::env::var("KITECONNECT_REQUEST_TOKEN").expect("KITECONNECT_REQUEST_TOKEN must be set");

    // Exchange the request token for an authenticated user session.
    let kite_session = client
        .session()
        .generate_session(&request_token)
        .await?
        .data
        .expect("expected session data in response");

    // Build streaming credentials and initial subscription state.
    let stream_creds = KiteStreamCredentials::from(kite_session);
    let stream_state = StreamState::from_credentials(stream_creds)
        .subscribe_token(Mode::Full, 408065) // INFY
        .subscribe_token(Mode::Full, 884737); // TATAMOTORS

    info!("Connecting ticker stream...");
    let mut ticker = WebSocketClient::connect(stream_state).await?;

    // Consume a limited number of messages and print them.
    for _ in 0..100 {
        if let Some(maybe_msg) = ticker.next().await {
            match maybe_msg {
                Ok(msg) => info!("Ticker message: {}", msg),
                Err(e) => error!("Ticker error: {}", e),
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "websocket"))]
fn main() {
    eprintln!(
        "This example requires the `websocket` feature.\n\
         Rebuild with:\n\
           cargo run -p manja --example ticker --features websocket"
    );
}
