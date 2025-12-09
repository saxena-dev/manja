use manja::UserSession;
use manja_ticker::{KiteStreamCredentials, Mode, StreamState};

mod support;
use support::read_to_object;

#[test]
fn facade_exposes_ticker_types_and_uri() {
    // Load a synthetic user session from fixture.
    let session: UserSession =
        read_to_object("./kiteconnect-mocks/generate_session.json");

    let creds = KiteStreamCredentials::from(session);
    let state = StreamState::from_credentials(creds)
        .subscribe_token(Mode::Full, 408065)
        .subscribe_token(Mode::Quote, 884737);

    let uri = state.to_uri();

    // Ensure the URI contains the expected query parameters.
    assert!(uri.contains("api_key="));
    assert!(uri.contains("access_token="));

    // Sanity check: ensure the base comes either from env or default constant.
    let expected_base = std::env::var("KITECONNECT_WSS_API_BASE")
        .unwrap_or_else(|_| manja_ticker::stream::KITECONNECT_WSS_API_BASE.to_string());
    assert!(uri.starts_with(&expected_base));
}
