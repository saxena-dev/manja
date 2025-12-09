use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mockito::ServerGuard;
use serde::de::DeserializeOwned;

use crate::client::HTTPClient;
use crate::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
use crate::credentials::KiteCredentials;
use crate::error::{Error, Result};
use manja_core::models::{KiteApiResponse, UserSession};

/// Resolve a fixture path relative to the crate or workspace root.
///
/// This helper first looks for the given `relative` path under
/// `CARGO_MANIFEST_DIR` (the crate root). If the file is not found there, it
/// falls back to the workspace root (one directory up).
pub fn resolve_fixture_path(relative: &str) -> PathBuf {
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let candidate = Path::new(&root).join(relative);
    if candidate.exists() {
        return candidate;
    }

    // When running in a workspace, fixture files may live at the workspace
    // root (one level up from the crate).
    let workspace_candidate = Path::new(&root).join("..").join(relative);
    if workspace_candidate.exists() {
        return workspace_candidate;
    }

    candidate
}

/// Read a Kite API fixture into the inner `data` model.
pub fn read_to_object<M>(path: &str) -> Result<M>
where
    M: DeserializeOwned,
{
    let full_path = resolve_fixture_path(path);
    let contents = std::fs::read_to_string(&full_path).unwrap_or_else(|err| {
        panic!("failed to read fixture at {}: {}", full_path.display(), err)
    });
    let obj: KiteApiResponse<M> =
        serde_json::from_str(&contents).map_err::<Error, _>(Into::into)?;
    obj.data
        .ok_or_else(|| Error::Internal("obj not found".to_string()))
}

pub type HTTPMethod = &'static str;
pub type APIEndpoint = &'static str;
pub type TestResponse = &'static str;

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

/// Construct an `HTTPClient` configured to talk to a mockito server, with a
/// preloaded `UserSession` fixture.
pub async fn get_http_test_client() -> (ServerGuard, HTTPClient) {
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
        read_to_object::<UserSession>("./kiteconnect-mocks/generate_session.json").unwrap();

    (server, HTTPClient::with_config(config).with_user_session(session))
}
