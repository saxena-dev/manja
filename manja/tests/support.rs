#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mockito::ServerGuard;
use serde::de::DeserializeOwned;

use manja::kite::connect::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
use manja::kite::connect::credentials::KiteCredentials;
use manja::{KiteApiResponse, ManjaClient, UserSession};

pub type HTTPMethod = &'static str;
pub type APIEndpoint = &'static str;
pub type TestResponse = &'static str;

/// Resolve a fixture path relative to the `manja` crate or workspace root.
pub fn resolve_fixture_path(relative: &str) -> PathBuf {
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let candidate = Path::new(&root).join(relative);
    if candidate.exists() {
        return candidate;
    }

    let workspace_candidate = Path::new(&root).join("..").join(relative);
    if workspace_candidate.exists() {
        return workspace_candidate;
    }

    candidate
}

/// Read a Kite API fixture into the inner `data` model.
pub fn read_to_object<M>(path: &str) -> M
where
    M: DeserializeOwned,
{
    let full_path = resolve_fixture_path(path);
    let contents = std::fs::read_to_string(&full_path).unwrap_or_else(|err| {
        panic!("failed to read fixture at {}: {}", full_path.display(), err)
    });
    let obj: KiteApiResponse<M> = serde_json::from_str(&contents).unwrap();
    obj.data
        .unwrap_or_else(|| panic!("fixture data missing in {}", full_path.display()))
}

/// Add a set of mocked responses to a mockito server.
pub async fn add_mocks(
    mut server: ServerGuard,
    mock_map: HashMap<(HTTPMethod, APIEndpoint), TestResponse>,
) -> ServerGuard {
    for ((method, api_endpoint), response_path) in mock_map {
        let full_path = resolve_fixture_path(response_path);
        let response_json = std::fs::read_to_string(&full_path).unwrap_or_else(|err| {
            panic!("failed to read fixture at {}: {}", full_path.display(), err)
        });
        let _m = server
            .mock(method, api_endpoint)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_json)
            .create_async()
            .await;
    }
    server
}

/// Construct a `ManjaClient` configured to talk to a mockito server, with a
/// preloaded `UserSession` fixture.
pub async fn make_manja_test_client() -> (ServerGuard, ManjaClient) {
    let server = mockito::Server::new_async().await;

    let credentials = KiteCredentials::new(
        "TEST_API_KEY",
        "TEST_API_SECRET",
        "TEST_USER_ID",
        "TEST_PASSWORD",
        "TEST_TOTP",
    );
    let config = Config::from_parts(
        server.url(),
        KITECONNECT_API_LOGIN.to_string(),
        KITECONNECT_API_REDIRECT.to_string(),
        credentials,
    );
    let session =
        read_to_object::<UserSession>("./kiteconnect-mocks/generate_session.json");

    let mut client = ManjaClient::new(config);
    client.http_mut().set_user_session(Some(session));

    (server, client)
}
