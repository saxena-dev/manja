//! Token exchange and invalidation, the only session operations.
//!
//! By default this runs against a local server serving the official
//! `generate_session.json` and `session_logout.json`:
//!
//! ```text
//! cargo run --example session
//! ```
//!
//! Against the live Kite API it runs only when asked explicitly, with inputs
//! you supply: the API key and request token as arguments, the API secret on
//! standard input so it never appears in the process list.
//!
//! ```text
//! cargo run --example session -- live exchange <api_key> <request_token>
//! cargo run --example session -- live invalidate <api_key> <access_token>
//! ```
//!
//! Obtaining the request token is up to you; the SDK has no flow for it.

#[path = "support/mod.rs"]
mod support;

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::{AccessToken, ApiKey, ApiSecret, RequestToken};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => local().await,
        ["live", "exchange", key, token] => {
            let mut secret = String::new();
            std::io::stdin().read_line(&mut secret)?;
            let client = HTTPClient::new(Config::default())?;
            let session = client
                .session(ApiKey::new(key)?)
                .exchange(&RequestToken::new(token)?, &ApiSecret::new(secret.trim())?)
                .await?
                .data
                .expect("data");
            // The tokens are secret-wrapped; exporting them is explicit.
            let _credentials = session.credentials()?;
            println!("exchanged a session for {}", session.user_id);
            Ok(())
        }
        ["live", "invalidate", key, token] => {
            let client = HTTPClient::new(Config::default())?;
            let done = client
                .session(ApiKey::new(key)?)
                .invalidate(&AccessToken::new(token)?)
                .await?;
            println!("invalidated: {:?}", done.data);
            Ok(())
        }
        _ => {
            eprintln!("usage: session [live exchange|invalidate <api_key> <token>]");
            std::process::exit(2);
        }
    }
}

async fn local() -> Result<(), Box<dyn std::error::Error>> {
    let base = support::http_mocks(&[
        ("POST", "/session/token", "generate_session.json"),
        ("DELETE", "/session/token", "session_logout.json"),
    ])
    .await;
    // Exchange is pre-session: the client needs no credentials, and the API
    // secret is borrowed for the checksum only. One attempt, never retried.
    let client = HTTPClient::new(Config::new(base))?;
    let session = client
        .session(ApiKey::new("api_key")?)
        .exchange(
            &RequestToken::new("request_token")?,
            &ApiSecret::new("api_secret")?,
        )
        .await?
        .data
        .expect("data");
    println!(
        "exchanged: user {} ({:?})",
        session.user_id, session.user_type
    );
    let credentials = session.credentials()?;
    // Invalidation takes the token to retire and no secret.
    let done = client
        .session(ApiKey::new("api_key")?)
        .invalidate(credentials.access_token())
        .await?;
    println!("invalidated: {:?}", done.data);
    Ok(())
}
