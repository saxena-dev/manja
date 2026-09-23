//! Self-tests for the hermetic test support in this directory. Each fault the
//! harnesses can inject is exercised here, and two tests drive `manja`'s own
//! clients through the harnesses to prove the seams are usable.
//!
//! Every negative input below is synthetic unless it is named as an official
//! `kiteconnect-mocks/` fixture; each one states what it was derived from.

#[path = "mod.rs"]
mod support;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::KiteCredentials;
use manja::kite::ticker::{Mode, StreamState, WebSocketClient};

use support::fixtures::{self, FixtureError};
use support::http::{refused_base_url, HttpHarness, Reply};
use support::ws::{Handshake, Step, WsConnection, WsHarness};

/// Bound on every wait below, so a harness defect fails a test instead of hanging it.
const WAIT: Duration = Duration::from_secs(5);

fn reqwest_client() -> reqwest::Client {
    reqwest::Client::builder().timeout(WAIT).build().unwrap()
}

/// A fresh directory under the system temp dir, unique to this test process and `tag`.
fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("manja-support-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// --- fixtures ---------------------------------------------------------------

#[test]
fn fixtures_resolve_from_the_manifest_dir() {
    let dir = fixtures::mocks_dir();
    assert!(dir.is_absolute(), "{}", dir.display());
    assert!(dir.starts_with(env!("CARGO_MANIFEST_DIR")));
    let text = fixtures::read("profile.json").unwrap();
    assert_eq!(
        text.as_bytes(),
        std::fs::read(dir.join("profile.json")).unwrap()
    );
}

#[test]
fn missing_fixture_error_names_the_resolved_path() {
    let err = fixtures::read("no_such_fixture.json").unwrap_err();
    let expected = fixtures::mocks_dir().join("no_such_fixture.json");
    assert!(matches!(err, FixtureError::Unreadable(ref p, _) if *p == expected));
    assert!(
        err.to_string().contains(&expected.display().to_string()),
        "{err}"
    );
}

#[test]
fn missing_fixture_dir_is_a_setup_failure() {
    let dir = scratch_dir("missing-dir").join("kiteconnect-mocks");
    let err = fixtures::read_in(&dir, "profile.json").unwrap_err();
    assert!(matches!(err, FixtureError::MissingDir(ref p) if *p == dir));
    let message = err.to_string();
    assert!(message.contains(&dir.display().to_string()), "{message}");
    assert!(message.contains("submodule"), "{message}");
}

#[test]
fn served_fixture_bytes_are_unchanged_by_the_json_check() {
    let raw = std::fs::read(fixtures::mocks_dir().join("margins.json")).unwrap();
    assert_eq!(fixtures::json_body("margins.json").unwrap().as_bytes(), raw);
}

#[test]
fn malformed_fixture_error_names_the_path() {
    // Synthetic: official profile.json truncated to its first 40 bytes.
    let dir = scratch_dir("malformed");
    let official = fixtures::read("profile.json").unwrap();
    std::fs::write(dir.join("profile.json"), &official.as_bytes()[..40]).unwrap();
    let err = fixtures::json_in::<serde_json::Value>(&dir, "profile.json").unwrap_err();
    assert!(matches!(err, FixtureError::Malformed(ref p, _) if *p == dir.join("profile.json")));
    assert!(err.to_string().starts_with("malformed fixture "), "{err}");
    std::fs::remove_dir_all(dir).unwrap();
}

// --- HTTP harness -----------------------------------------------------------

#[tokio::test]
async fn http_serves_official_body_unchanged_and_records_the_request() {
    let official = fixtures::json_body("profile.json").unwrap();
    let harness = HttpHarness::start(vec![Reply::json(official.clone())]).await;
    let response = reqwest_client()
        .post(format!("{}/probe?x=1", harness.base_url()))
        .header("X-Kite-Version", "3")
        .body("a=b")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap(), official.as_bytes());
    let [request] = harness.requests().try_into().unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/probe?x=1");
    assert_eq!(request.header("x-kite-version"), Some("3"));
    assert_eq!(request.body, b"a=b");
}

#[tokio::test]
async fn http_malformed_body_is_delivered_verbatim() {
    // Synthetic: a JSON envelope cut off inside `data`.
    let malformed = br#"{"status": "success", "data": {"user_id": "#.to_vec();
    let harness = HttpHarness::start(vec![Reply::json(malformed.clone())]).await;
    let body = reqwest_client()
        .get(harness.base_url())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(body, malformed);
    assert!(serde_json::from_slice::<serde_json::Value>(&body).is_err());
}

#[tokio::test]
async fn http_response_loss_is_an_error() {
    let harness = HttpHarness::start(vec![Reply::DropAfterRequest]).await;
    let result = reqwest_client().get(harness.base_url()).send().await;
    assert!(result.is_err(), "{result:?}");
    assert_eq!(harness.requests().len(), 1, "the request did arrive");
}

#[tokio::test]
async fn http_close_on_accept_is_an_error_before_any_request_is_read() {
    let harness = HttpHarness::start(vec![Reply::CloseOnAccept]).await;
    let result = reqwest_client().get(harness.base_url()).send().await;
    assert!(result.is_err(), "{result:?}");
    assert!(harness.requests().is_empty());
}

#[tokio::test]
async fn http_eof_mid_body_is_an_error() {
    let body = fixtures::json_body("profile.json").unwrap().into_bytes();
    let harness = HttpHarness::start(vec![Reply::TruncateBody {
        status: 200,
        body,
        sent: 10,
    }])
    .await;
    let response = reqwest_client()
        .get(harness.base_url())
        .send()
        .await
        .unwrap();
    assert!(response.bytes().await.is_err());
}

#[tokio::test]
async fn http_stalled_peer_times_out() {
    let harness = HttpHarness::start(vec![Reply::Stall]).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let err = client.get(harness.base_url()).send().await.unwrap_err();
    assert!(err.is_timeout(), "{err:?}");
}

#[tokio::test]
async fn http_refused_connection_is_a_connect_error() {
    let err = reqwest_client()
        .get(refused_base_url().await)
        .send()
        .await
        .unwrap_err();
    assert!(err.is_connect(), "{err:?}");
}

fn manja_client(base_url: String) -> HTTPClient {
    let config = Config::from_parts(
        base_url.clone(),
        base_url.clone(),
        base_url,
        KiteCredentials::new("test_api_key", "", "", ""),
    );
    HTTPClient::with_config(config).unwrap()
}

#[tokio::test]
async fn manja_http_client_reads_official_profile_through_the_harness() {
    let harness = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("profile.json").unwrap(),
    )])
    .await;
    let response = manja_client(harness.base_url())
        .user()
        .profile()
        .await
        .unwrap();
    // Expected values are written out from the official fixture, not re-parsed from it.
    let profile = serde_json::to_value(response.data.unwrap()).unwrap();
    assert_eq!(profile["user_id"], "AB1234");
    assert_eq!(profile["user_name"], "AxAx Bxx");
    assert_eq!(profile["broker"], "ZERODHA");
    assert_eq!(profile["exchanges"][0], "BFO");
    assert_eq!(profile["exchanges"].as_array().unwrap().len(), 8);
    let [request] = harness.requests().try_into().unwrap();
    assert_eq!(
        (request.method.as_str(), request.target.as_str()),
        ("GET", "/user/profile")
    );
    assert_eq!(request.header("x-kite-version"), Some("3"));
}

#[tokio::test]
async fn manja_http_client_reports_response_loss_as_an_error() {
    let harness = HttpHarness::start(vec![Reply::DropAfterRequest]).await;
    let result = manja_client(harness.base_url()).user().profile().await;
    assert!(result.is_err(), "response loss must not become success");
}

// --- WebSocket harness ------------------------------------------------------

async fn connect(
    harness: &WsHarness,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (ws, _) = tokio_tungstenite::connect_async(format!("{}/probe?a=1", harness.url()))
        .await
        .unwrap();
    ws
}

async fn next<S: StreamExt + Unpin>(stream: &mut S) -> Option<S::Item> {
    tokio::time::timeout(WAIT, stream.next()).await.unwrap()
}

#[tokio::test]
async fn ws_delivers_payloads_verbatim_and_records_traffic() {
    // Synthetic: 7 bytes, shorter than any packet header, standing in for a truncated packet.
    let truncated = vec![0x00, 0x01, 0x00, 0x08, 0x00, 0x06, 0x3a];
    let harness = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: vec![
            Step::Send(Message::Binary(truncated.clone())),
            Step::Send(Message::Text("{not json".into())),
        ],
    }])
    .await;
    let mut ws = connect(&harness).await;
    assert_eq!(
        next(&mut ws).await.unwrap().unwrap(),
        Message::Binary(truncated)
    );
    assert_eq!(
        next(&mut ws).await.unwrap().unwrap(),
        Message::Text("{not json".into())
    );
    ws.send(Message::Text("client-hello".into())).await.unwrap();
    tokio::time::timeout(WAIT, async {
        while harness.messages().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        harness.messages(),
        vec![Message::Text("client-hello".into())]
    );
    assert_eq!(harness.handshakes()[0].target, "/probe?a=1");
}

#[tokio::test]
async fn ws_rejected_handshake_is_an_http_error() {
    let harness = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Reject(403),
        steps: vec![],
    }])
    .await;
    let err = tokio_tungstenite::connect_async(harness.url())
        .await
        .unwrap_err();
    assert!(
        matches!(err, WsError::Http(ref r) if r.status() == 403),
        "{err:?}"
    );
    assert_eq!(harness.handshakes().len(), 1);
}

#[tokio::test]
async fn ws_close_frame_is_delivered() {
    let harness = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: vec![Step::Close],
    }])
    .await;
    let mut ws = connect(&harness).await;
    assert!(matches!(next(&mut ws).await, Some(Ok(Message::Close(_)))));
}

#[tokio::test]
async fn ws_eof_without_close_frame_is_not_a_clean_close() {
    let harness = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: vec![Step::Eof],
    }])
    .await;
    let mut ws = connect(&harness).await;
    let item = next(&mut ws).await;
    assert!(matches!(item, None | Some(Err(_))), "{item:?}");
}

#[tokio::test]
async fn ws_send_after_peer_eof_fails() {
    let harness = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: vec![Step::Eof],
    }])
    .await;
    let mut ws = connect(&harness).await;
    // The first writes can land in the local socket buffer; a later one must fail.
    let mut failed = false;
    for _ in 0..50 {
        if ws.send(Message::Text("x".into())).await.is_err() {
            failed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        failed,
        "sending to a peer that dropped the connection never failed"
    );
}

#[tokio::test]
async fn ws_stalled_peer_sends_nothing() {
    let harness = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: vec![],
    }])
    .await;
    let mut ws = connect(&harness).await;
    let silent = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
    assert!(silent.is_err(), "a stalled peer delivered {silent:?}");
}

#[tokio::test]
async fn manja_ticker_connects_through_the_harness_and_receives_raw_frames() {
    // Synthetic: 8 bytes laid out like an LTP packet for instrument token 408065.
    let frame = vec![0x00, 0x06, 0x3a, 0x01, 0x00, 0x02, 0x66, 0x83];
    let harness = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: vec![Step::Send(Message::Binary(frame.clone()))],
    }])
    .await;
    let state = StreamState::from_parts(
        harness.url(),
        "test_api_key".into(),
        "test_access_token".into(),
    )
    .subscribe_token(Mode::LTP, 408065);
    let mut ticker = tokio::time::timeout(WAIT, WebSocketClient::connect(state))
        .await
        .unwrap()
        .unwrap();
    let message = next(&mut ticker).await.unwrap().unwrap();
    assert_eq!(message, Message::Binary(frame));
    let target = &harness.handshakes()[0].target;
    assert!(target.contains("api_key=test_api_key"), "{target}");
    assert!(
        target.contains("access_token=test_access_token"),
        "{target}"
    );
}
