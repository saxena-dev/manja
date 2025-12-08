use std::io::{self, Write};

use manja::{KiteApiResponse, ManjaClient, UserMargins, UserProfile};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize a simple logger for the example.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    // Construct a client from environment-based configuration.
    //
    // Required environment variables (see crate docs for details):
    // - KITECONNECT_API_KEY
    // - KITECONNECT_API_SECRET
    // - KITECONNECT_USER_ID
    // - KITECONNECT_PASSWORD
    // - KITECONNECT_TOTP_KEY
    let mut client = ManjaClient::from_env();

    // Obtain the request token for this session.
    //
    // In a real application, this token is obtained from the redirect URL
    // after a successful interactive login in the browser. For this example
    // we first try to read it from the `KITECONNECT_REQUEST_TOKEN` environment
    // variable and, if missing, fall back to a simple stdin prompt.
    let request_token = std::env::var("KITECONNECT_REQUEST_TOKEN").unwrap_or_else(|_| {
        print!("Enter Kite Connect request_token: ");
        io::stdout().flush().expect("failed to flush stdout");

        let mut token = String::new();
        io::stdin()
            .read_line(&mut token)
            .expect("failed to read request_token");

        token.trim().to_string()
    });

    // Complete the second step of the login flow by exchanging the
    // `request_token` for an access token and user session.
    let session: KiteApiResponse<_> = client.session().generate_session(&request_token).await?;
    println!("Logged in, session status = {}", session.status);

    // Fetch the user profile.
    let profile: KiteApiResponse<UserProfile> = client.user().profile().await?;
    println!("User profile: {:?}", profile.data);

    // Fetch overall user margins.
    let margins: KiteApiResponse<UserMargins> = client.user().margins().await?;
    println!("User margins: {:?}", margins.data);

    Ok(())
}

