//! The required session operations: pre-session token
//! exchange that borrows the API secret, and invalidation that takes none.
//!
//! Baselines are the official `generate_session.json` and
//! `session_logout.json`, served unchanged; the checksum vector was computed
//! independently with `shasum -a 256` over
//! `test_api_key` + `request_token_0123` + `test_api_secret`
//! (`kite:user.md:9,97-99`). Sentinel and fault
//! responses are labelled supplements.

mod support;

use std::time::Duration;

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::{
    AccessToken, ApiKey, ApiSecret, Credentials, RequestToken,
};
use manja::kite::connect::dispatch::ReceiptIds;
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpError, HttpErrorKind, TransportStage};
use manja::kite::obs::schema::{Endpoint, HttpOperationResult};

use support::fixtures;
use support::http::{HttpHarness, RecordedRequest, Reply};

const CHECKSUM: &str = "a8d35625a57e864a7f4dea0c182514394272ed642a506ec499e9822c6bb88ae7";
const SECRET: &str = "test_api_secret";
const REQUEST_TOKEN: &str = "request_token_0123";

fn unauthenticated(base: &str) -> HTTPClient {
    unauthenticated_with(base, SchedulerLimits::default())
}

fn unauthenticated_with(base: &str, scheduler: SchedulerLimits) -> HTTPClient {
    let config = Config::new(base).with_limits(HttpLimits::default().with_scheduler(scheduler));
    HTTPClient::new(config).unwrap()
}

fn key() -> ApiKey {
    ApiKey::new("test_api_key").unwrap()
}

fn only(h: &HttpHarness) -> RecordedRequest {
    let [r] = h.requests().try_into().unwrap();
    r
}

async fn exchange(c: &HTTPClient) -> Result<(), HttpError> {
    c.session(key())
        .exchange(
            &RequestToken::new(REQUEST_TOKEN).unwrap(),
            &ApiSecret::new(SECRET).unwrap(),
        )
        .await
        .map(|_| ())
}

#[tokio::test]
async fn exchange_needs_no_access_token_and_sends_only_the_documented_fields() {
    let h = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("generate_session.json").unwrap(),
    )])
    .await;
    let c = unauthenticated(&h.base_url());
    assert!(c.credentials().is_none());
    let session = c
        .session(key())
        .exchange(
            &RequestToken::new(REQUEST_TOKEN).unwrap(),
            &ApiSecret::new(SECRET).unwrap(),
        )
        .await
        .unwrap()
        .data;
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        ("POST", "/session/token")
    );
    assert_eq!(r.header("x-kite-version"), Some("3"));
    assert_eq!(r.header("authorization"), None);
    assert_eq!(
        r.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    let body = String::from_utf8(r.body).unwrap();
    assert_eq!(
        body,
        format!("api_key=test_api_key&request_token={REQUEST_TOKEN}&checksum={CHECKSUM}")
    );
    assert!(!body.contains(SECRET), "the secret is not a form field");
    // The official response, with independent field assertions.
    assert_eq!(session.user_id, "XX0000");
    assert_eq!(session.user_type, "individual");
    assert_eq!(session.broker, "ZERODHA");
    assert_eq!(session.exchanges.len(), 8);
    assert!(
        session.refresh_token.is_none(),
        "an empty refresh token is None"
    );
    assert!(session.enctoken.is_some());
    assert_eq!(
        session.login_time.unwrap().to_rfc3339(),
        "2021-01-01T16:15:14+05:30"
    );
    assert_eq!(session.meta.unwrap().demat_consent, "physical");
    // Nothing was installed into the client.
    assert!(c.credentials().is_none());
}

#[tokio::test]
async fn exchange_never_attaches_an_unrelated_access_token() {
    let h = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("generate_session.json").unwrap(),
    )])
    .await;
    let authenticated = unauthenticated(&h.base_url())
        .with_credentials(Credentials::new("other_key", "other_token").unwrap());
    exchange(&authenticated).await.unwrap();
    let r = only(&h);
    assert_eq!(r.header("authorization"), None);
    assert!(!String::from_utf8(r.body).unwrap().contains("other_token"));
    // The client's own snapshot is unchanged.
    assert_eq!(
        authenticated
            .credentials()
            .unwrap()
            .access_token()
            .expose_secret(),
        "other_token"
    );
}

#[tokio::test]
async fn invalidation_takes_the_target_token_and_no_secret() {
    let h = HttpHarness::start(vec![Reply::json(
        fixtures::json_body("session_logout.json").unwrap(),
    )])
    .await;
    let c = unauthenticated(&h.base_url())
        .with_credentials(Credentials::new("test_api_key", "client_token").unwrap());
    let invalidated = c
        .session(key())
        .invalidate(&AccessToken::new("target_token").unwrap())
        .await
        .unwrap()
        .data;
    assert!(invalidated);
    let r = only(&h);
    assert_eq!(
        (r.method.as_str(), r.target.as_str()),
        (
            "DELETE",
            "/session/token?api_key=test_api_key&access_token=target_token"
        )
    );
    assert_eq!(
        r.header("authorization"),
        None,
        "no substituted client token"
    );
    assert!(r.body.is_empty());
    // The local client keeps its snapshot until the caller retires it.
    assert_eq!(
        c.credentials().unwrap().access_token().expose_secret(),
        "client_token"
    );
}

#[tokio::test]
async fn every_fault_is_one_attempt_with_stage_evidence() {
    // Supplemental faults for the exchange: 429 envelope, response loss,
    // an HTML success page, and a stalled peer under a 1 s session deadline.
    let rate_limited = || Reply::Respond {
        status: 429,
        content_type: "application/json",
        body:
            br#"{"status":"error","message":"Too many requests","error_type":"NetworkException"}"#
                .to_vec(),
    };
    let cases: Vec<(Vec<Reply>, HttpErrorKind, TransportStage)> = vec![
        (
            vec![rate_limited(), rate_limited()],
            HttpErrorKind::Broker,
            TransportStage::ResponseReceived,
        ),
        (
            vec![Reply::DropAfterRequest, Reply::DropAfterRequest],
            HttpErrorKind::Transport,
            TransportStage::Started,
        ),
        (
            vec![Reply::Respond {
                status: 200,
                content_type: "text/html",
                body: b"<html>ok</html>".to_vec(),
            }],
            HttpErrorKind::Decode,
            TransportStage::ResponseReceived,
        ),
        (
            vec![Reply::Stall, Reply::Stall],
            HttpErrorKind::Transport,
            TransportStage::Started,
        ),
    ];
    let short = SchedulerLimits::default()
        .with_session_deadline(Duration::from_secs(1))
        .unwrap();
    for (replies, kind, stage) in cases {
        let h = HttpHarness::start(replies).await;
        let c = unauthenticated_with(&h.base_url(), short.clone());
        let err = exchange(&c).await.unwrap_err();
        let e = &err;
        assert_eq!((e.kind(), e.stage()), (kind, stage), "{e}");
        assert_eq!(h.requests().len(), 1, "one attempt: {e}");
        for rendered in [err.to_string(), format!("{err:?}")] {
            assert!(!rendered.contains(SECRET), "{rendered}");
            assert!(!rendered.contains(REQUEST_TOKEN), "{rendered}");
            assert!(!rendered.contains(CHECKSUM), "{rendered}");
        }
    }
}

#[tokio::test]
async fn dropping_the_exchange_future_abandons_nothing() {
    // The server answers after 300 ms; the caller gives up after 100 ms.
    // The exchange was dispatched, so its attempt runs to completion and
    // its outcome lands in the orphan ring; nothing is recorded as cancelled.
    let h = HttpHarness::start(vec![Reply::Delayed {
        after: Duration::from_millis(300),
        body: fixtures::json_body("generate_session.json")
            .unwrap()
            .into_bytes(),
    }])
    .await;
    let c = unauthenticated(&h.base_url());
    let dropped = tokio::time::timeout(Duration::from_millis(100), exchange(&c)).await;
    assert!(dropped.is_err());
    assert_eq!(h.requests().len(), 1);
    assert_eq!(c.diagnostics().dispatches_in_flight, 1);
    c.close(Duration::from_secs(5)).await.unwrap();
    let d = c.diagnostics();
    assert!(
        d.last_failures.is_empty(),
        "nothing failed: {:?}",
        d.last_failures
    );
    assert_eq!(d.dispatches_in_flight, 0);
    assert_eq!(d.orphaned_outcome_count, 1);
    let [outcome] = d.orphaned_outcomes.as_slice() else {
        panic!("one orphaned outcome: {:?}", d.orphaned_outcomes);
    };
    assert_eq!(outcome.endpoint, Endpoint::SessionToken);
    assert_eq!(outcome.result, HttpOperationResult::Ok);
    assert_eq!(outcome.http_status, Some(200));
    assert_eq!(outcome.stage, TransportStage::ResponseReceived);
    assert_eq!(outcome.receipt, ReceiptIds::default());
    // The outcome record carries no token: it holds the stage, the result
    // and the hash, never a body.
    let rendered = format!("{outcome:?}");
    for s in [SECRET, REQUEST_TOKEN, CHECKSUM, "XXXXXX"] {
        assert!(!rendered.contains(s), "{rendered}");
    }
}

#[tokio::test]
async fn returned_tokens_are_secret_wrapped_and_exported_intentionally() {
    // Supplemental, derived from generate_session.json: sentinel tokens.
    let body = fixtures::json_body("generate_session.json")
        .unwrap()
        .replace(
            "\"access_token\": \"XXXXXX\"",
            "\"access_token\": \"SENTINELaccess0001\"",
        )
        .replace(
            "\"public_token\": \"XXXXXXXX\"",
            "\"public_token\": \"SENTINELpublic0002\"",
        )
        .replace(
            "\"api_key\": \"XXXXXX\"",
            "\"api_key\": \"SENTINELkey0003\"",
        );
    let h = HttpHarness::start(vec![Reply::json(body)]).await;
    let c = unauthenticated(&h.base_url());
    let session = c
        .session(key())
        .exchange(
            &RequestToken::new(REQUEST_TOKEN).unwrap(),
            &ApiSecret::new(SECRET).unwrap(),
        )
        .await
        .unwrap()
        .data;
    let rendered = format!("{session:?}");
    for sentinel in [
        "SENTINELaccess0001",
        "SENTINELpublic0002",
        "SENTINELkey0003",
    ] {
        assert!(!rendered.contains(sentinel), "{rendered}");
    }
    let creds = session.credentials().unwrap();
    assert_eq!(creds.api_key().as_str(), "SENTINELkey0003");
    assert_eq!(creds.access_token().expose_secret(), "SENTINELaccess0001");
    // Nothing about the exchange is retained by the client.
    let retained = format!("{:?} {:?}", c, c.diagnostics());
    for s in [SECRET, REQUEST_TOKEN, CHECKSUM, "SENTINELaccess0001"] {
        assert!(!retained.contains(s), "{retained}");
    }
}

#[tokio::test]
async fn ordinary_calls_and_drop_never_touch_the_session_endpoint() {
    let h = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("profile.json").unwrap()),
        Reply::json(fixtures::json_body("margins.json").unwrap()),
    ])
    .await;
    let c = unauthenticated(&h.base_url())
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap());
    c.user().profile().await.unwrap();
    c.user().margins().await.unwrap();
    let derived = c.with_credentials(Credentials::new("k2", "t2").unwrap());
    drop(derived);
    drop(c);
    tokio::task::yield_now().await;
    let targets: Vec<String> = h.requests().into_iter().map(|r| r.target).collect();
    assert_eq!(targets, ["/user/profile", "/user/margins"]);
}

// ---- wire forms --------------------------------------------------------

fn sha(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}

#[tokio::test]
async fn session_operations_send_exactly_their_wire_forms_and_render_no_secret() {
    use manja::kite::connect::wire::WireForm;
    let h = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("generate_session.json").unwrap()),
        Reply::json(fixtures::json_body("session_logout.json").unwrap()),
    ])
    .await;
    let c = unauthenticated(&h.base_url());
    let token = RequestToken::new(REQUEST_TOKEN).unwrap();
    let secret = ApiSecret::new(SECRET).unwrap();
    let exchange = WireForm::session_exchange(&key(), &token, &secret);
    c.session(key()).exchange(&token, &secret).await.unwrap();
    let access = AccessToken::new("target_token").unwrap();
    let invalidate = WireForm::session_invalidation(&key(), &access);
    c.session(key()).invalidate(&access).await.unwrap();
    let [r1, r2] = h.requests().try_into().unwrap();

    // Exchange: the captured body is the form's bytes, hash for hash.
    assert_eq!(r1.method, "POST");
    assert_eq!(r1.target, exchange.path());
    assert_eq!(r1.header("content-type"), exchange.content_type());
    assert_eq!(sha(&r1.body), exchange.sha256());
    assert_eq!(
        sha(
            format!("api_key=test_api_key&request_token={REQUEST_TOKEN}&checksum={CHECKSUM}")
                .as_bytes()
        ),
        exchange.sha256()
    );
    // Its rendering carries the field names but not the token or checksum.
    let shown = exchange.to_string();
    let debug = format!("{exchange:?}");
    assert_eq!(
        shown,
        "api_key=test_api_key&request_token=<redacted>&checksum=<redacted>"
    );
    for s in [REQUEST_TOKEN, CHECKSUM, SECRET] {
        assert!(!shown.contains(s), "{shown}");
        assert!(!debug.contains(s), "{debug}");
    }

    // Invalidation: no body, the query is sent as the form renders it.
    assert_eq!(r2.method, "DELETE");
    assert_eq!(
        r2.target,
        "/session/token?api_key=test_api_key&access_token=target_token"
    );
    assert_eq!(invalidate.path(), "/session/token");
    assert_eq!(invalidate.body_len(), 0);
    assert_eq!(invalidate.content_type(), None);
    assert_eq!(sha(&r2.body), invalidate.sha256());
    assert_eq!(invalidate.to_string(), "");
    let debug = format!("{invalidate:?}");
    assert!(debug.contains("access_token=<redacted>"), "{debug}");
    assert!(!debug.contains("target_token"), "{debug}");
    assert_eq!(
        exchange.sha256(),
        WireForm::session_exchange(&key(), &token, &secret).sha256()
    );
}
