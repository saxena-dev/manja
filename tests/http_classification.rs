//! Total HTTP success/error classification (plan task S04), exercised
//! through the loopback harness in `tests/support/http.rs`.
//!
//! Success baselines are the official `kiteconnect-mocks/` fixtures served
//! unchanged. Every error body below is a labelled supplemental fixture
//! (INV-GAP-07): the official corpus has no non-2xx envelopes, so each one
//! states the documented shape it follows
//! (`kite-api-docs/docs/connect/v3/response-structure.md:17-28`,
//! `exceptions.md:7-16`).

mod support;

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpError, HttpErrorKind, KiteApiException, ManjaError, TransportStage};
use manja::kite::obs::schema::{Endpoint, Method};
use manja::kite::protocol::Inbound;

use support::fixtures;
use support::http::{refused_base_url, HttpHarness, Reply};

const SENTINEL: &str = "SENTINELaccessTOKEN0123456789abcdef";

fn client(base: &str, limits: HttpLimits) -> HTTPClient {
    let config = Config::new(base).with_limits(limits);
    HTTPClient::with_config(config)
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", SENTINEL).unwrap())
}

fn reply(status: u16, content_type: &'static str, body: &str) -> Reply {
    Reply::Respond {
        status,
        content_type,
        body: body.as_bytes().to_vec(),
    }
}

/// Classification is tested one attempt at a time; retry policy is
/// exercised in tests/http_scheduling.rs.
fn one_attempt() -> HttpLimits {
    HttpLimits::default().with_scheduler(SchedulerLimits::default().with_read_attempts(1).unwrap())
}

async fn profile_with(replies: Vec<Reply>) -> (Result<(), ManjaError>, HttpHarness) {
    let harness = HttpHarness::start(replies).await;
    let result = client(&harness.base_url(), one_attempt())
        .user()
        .profile()
        .await
        .map(|_| ());
    (result, harness)
}

fn http(err: &ManjaError) -> &HttpError {
    err.as_http()
        .unwrap_or_else(|| panic!("not an HTTP error: {err:?}"))
}

/// Every rendered form of an error: no secret and no URL. Endpoint
/// templates and a transport source's socket address are permitted.
fn assert_clean(err: &ManjaError) {
    for rendered in [err.to_string(), format!("{err:?}")] {
        assert!(!rendered.contains(SENTINEL), "{rendered}");
        assert!(!rendered.contains("http://"), "{rendered}");
    }
}

#[tokio::test]
async fn official_success_envelope_is_ok() {
    let body = fixtures::json_body("profile.json").unwrap();
    let (result, harness) = profile_with(vec![Reply::json(body)]).await;
    result.unwrap();
    let [request] = harness.requests().try_into().unwrap();
    assert_eq!(request.header("x-kite-version"), Some("3"));
    assert_eq!(
        request.header("authorization").map(|v| v.len()),
        Some(format!("token test_api_key:{SENTINEL}").len())
    );
}

struct Case {
    name: &'static str,
    status: u16,
    content_type: &'static str,
    body: &'static str,
    kind: HttpErrorKind,
    error_type: Option<Option<&'static str>>,
}

#[tokio::test]
async fn error_matrix_never_yields_ok_and_keeps_the_status() {
    let cases = [
        Case {
            // Supplemental: documented error envelope on a 200 status.
            name: "200 with status=error",
            status: 200,
            content_type: "application/json",
            body: r#"{"status":"error","message":"Invalid input","error_type":"InputException"}"#,
            kind: HttpErrorKind::Broker,
            error_type: Some(Some("InputException")),
        },
        Case {
            name: "400 InputException",
            status: 400,
            content_type: "application/json",
            body: r#"{"status":"error","message":"Missing param","error_type":"InputException"}"#,
            kind: HttpErrorKind::Broker,
            error_type: Some(Some("InputException")),
        },
        Case {
            name: "403 TokenException",
            status: 403,
            content_type: "application/json",
            body: r#"{"status":"error","message":"Incorrect `api_key` or `access_token`.","error_type":"TokenException"}"#,
            kind: HttpErrorKind::AuthRejected,
            error_type: Some(Some("TokenException")),
        },
        Case {
            // The reviewed defect: a missing error_type used to panic.
            name: "500 without error_type",
            status: 500,
            content_type: "application/json",
            body: r#"{"status":"error","message":"Something unexpected"}"#,
            kind: HttpErrorKind::Broker,
            error_type: Some(None),
        },
        Case {
            name: "500 GeneralException",
            status: 500,
            content_type: "application/json",
            body: r#"{"status":"error","message":"Error message","error_type":"GeneralException"}"#,
            kind: HttpErrorKind::Broker,
            error_type: Some(Some("GeneralException")),
        },
        Case {
            name: "502 HTML gateway page",
            status: 502,
            content_type: "text/html",
            body: "<html><body>Bad Gateway</body></html>",
            kind: HttpErrorKind::HttpStatus,
            error_type: None,
        },
        Case {
            name: "504 empty body",
            status: 504,
            content_type: "text/plain",
            body: "",
            kind: HttpErrorKind::HttpStatus,
            error_type: None,
        },
        Case {
            name: "400 unknown error_type",
            status: 400,
            content_type: "application/json",
            body: r#"{"status":"error","message":"new","error_type":"FutureException"}"#,
            kind: HttpErrorKind::Broker,
            error_type: Some(Some("FutureException")),
        },
        Case {
            name: "200 malformed JSON",
            status: 200,
            content_type: "application/json",
            body: r#"{"status": "success", "data": {"user_id": "#,
            kind: HttpErrorKind::Decode,
            error_type: None,
        },
        Case {
            name: "200 HTML",
            status: 200,
            content_type: "text/html",
            body: "<html>maintenance</html>",
            kind: HttpErrorKind::Decode,
            error_type: None,
        },
        Case {
            name: "200 missing payload",
            status: 200,
            content_type: "application/json",
            body: r#"{"status":"success"}"#,
            kind: HttpErrorKind::Decode,
            error_type: None,
        },
        Case {
            name: "200 null payload",
            status: 200,
            content_type: "application/json",
            body: r#"{"status":"success","data":null}"#,
            kind: HttpErrorKind::Decode,
            error_type: None,
        },
        Case {
            name: "200 envelope without status",
            status: 200,
            content_type: "application/json",
            body: r#"{"data":{"user_id":"AB1234"}}"#,
            kind: HttpErrorKind::Decode,
            error_type: None,
        },
        Case {
            name: "200 payload of the wrong shape",
            status: 200,
            content_type: "application/json",
            body: r#"{"status":"success","data":[1,2,3]}"#,
            kind: HttpErrorKind::Decode,
            error_type: None,
        },
    ];
    for case in cases {
        let (result, _h) =
            profile_with(vec![reply(case.status, case.content_type, case.body)]).await;
        let err = result.expect_err(case.name);
        let e = http(&err);
        assert_eq!(e.kind(), case.kind, "{}", case.name);
        assert_eq!(e.http_status(), Some(case.status), "{}", case.name);
        assert_eq!(e.stage(), TransportStage::ResponseReceived, "{}", case.name);
        assert_eq!(e.method(), Method::Get);
        assert_eq!(e.endpoint(), Endpoint::UserProfile);
        assert_eq!(e.attempt(), 1, "{}", case.name);
        match case.error_type {
            Some(expected) => {
                let broker = e.broker().expect(case.name);
                assert_eq!(
                    broker.error_type().map(|t| t.as_wire().to_string()),
                    expected.map(str::to_string),
                    "{}",
                    case.name
                );
                assert!(broker.message().is_some(), "{}", case.name);
            }
            None => assert!(e.broker().is_none(), "{}", case.name),
        }
        assert_clean(&err);
    }
}

#[tokio::test]
async fn known_and_unknown_error_types_are_distinguished() {
    let (result, _h) = profile_with(vec![reply(
        403,
        "application/json",
        r#"{"status":"error","message":"expired","error_type":"TokenException"}"#,
    )])
    .await;
    let err = result.unwrap_err();
    assert_eq!(
        http(&err).broker().unwrap().error_type(),
        Some(&Inbound::Known(KiteApiException::TokenException))
    );
    let (result, _h) = profile_with(vec![reply(
        400,
        "application/json",
        r#"{"status":"error","error_type":"FutureException"}"#,
    )])
    .await;
    let err = result.unwrap_err();
    assert!(http(&err)
        .broker()
        .unwrap()
        .error_type()
        .unwrap()
        .is_unknown());
}

#[tokio::test]
async fn oversized_body_is_a_bounded_decode_error() {
    // Supplemental: a valid envelope padded past a 64 KiB bound.
    let body = format!(
        r#"{{"status":"success","data":{{"pad":"{}"}}}}"#,
        "x".repeat(70 * 1024)
    );
    let harness = HttpHarness::start(vec![Reply::json(body)]).await;
    let limits = HttpLimits::default()
        .with_json_body_bytes(64 * 1024)
        .unwrap();
    let err = client(&harness.base_url(), limits)
        .user()
        .profile()
        .await
        .unwrap_err();
    let e = http(&err);
    assert_eq!(e.kind(), HttpErrorKind::Decode);
    assert_eq!(e.http_status(), Some(200));
    assert!(e.detail().unwrap().as_str().contains("bound"));
}

#[tokio::test]
async fn truncated_body_is_a_transport_failure_after_the_response_started() {
    let body = fixtures::json_body("profile.json").unwrap().into_bytes();
    let (result, _h) = profile_with(vec![Reply::TruncateBody {
        status: 200,
        body,
        sent: 20,
    }])
    .await;
    let err = result.unwrap_err();
    let e = http(&err);
    assert_eq!(e.kind(), HttpErrorKind::Transport);
    assert_eq!(e.stage(), TransportStage::ResponseReceived);
    assert_eq!(e.http_status(), Some(200));
    assert_clean(&err);
}

#[tokio::test]
async fn response_loss_does_not_claim_the_request_was_not_sent() {
    let (result, harness) = profile_with(vec![Reply::DropAfterRequest]).await;
    let err = result.unwrap_err();
    let e = http(&err);
    assert_eq!(e.kind(), HttpErrorKind::Transport);
    assert_eq!(e.stage(), TransportStage::Started);
    assert!(e.may_have_reached_broker());
    assert_eq!(e.http_status(), None);
    assert_eq!(harness.requests().len(), 1, "the request did arrive");
    assert_clean(&err);
}

#[tokio::test]
async fn connect_failure_is_affirmative_evidence_of_no_dispatch() {
    let base = refused_base_url().await;
    let err = client(&base, one_attempt())
        .user()
        .profile()
        .await
        .unwrap_err();
    let e = http(&err);
    assert_eq!(e.kind(), HttpErrorKind::Transport);
    assert_eq!(e.stage(), TransportStage::NotStarted);
    assert!(!e.may_have_reached_broker());
    assert_clean(&err);
}

#[tokio::test]
async fn invalid_base_url_fails_before_transport() {
    let err = client("not a url", HttpLimits::default())
        .user()
        .profile()
        .await
        .unwrap_err();
    let e = http(&err);
    assert_eq!(e.kind(), HttpErrorKind::Configuration);
    assert_eq!(e.stage(), TransportStage::NotStarted);
}

#[tokio::test]
async fn broker_messages_are_sanitized_before_retention() {
    let body = format!(
        r#"{{"status":"error","message":"bad access_token={SENTINEL}","error_type":"TokenException"}}"#
    );
    let harness = HttpHarness::start(vec![Reply::Respond {
        status: 403,
        content_type: "application/json",
        body: body.into_bytes(),
    }])
    .await;
    let err = client(&harness.base_url(), HttpLimits::default())
        .user()
        .profile()
        .await
        .unwrap_err();
    assert_clean(&err);
    let message = http(&err).broker().unwrap().message().unwrap().as_str();
    assert!(message.starts_with("bad access_token="), "{message}");
}

#[tokio::test]
async fn rate_limited_response_is_an_error_with_its_status() {
    // Supplemental: documented error envelope with HTTP 429
    // (exceptions.md:39). One read attempt is allowed so the classified 429
    // itself is observed; retries are covered in tests/http_scheduling.rs.
    let body =
        r#"{"status":"error","message":"Too many requests","error_type":"NetworkException"}"#;
    let harness = HttpHarness::start(vec![reply(429, "application/json", body)]).await;
    let err = client(&harness.base_url(), one_attempt())
        .user()
        .profile()
        .await
        .unwrap_err();
    let e = http(&err);
    assert_eq!(e.http_status(), Some(429));
    assert_eq!(e.kind(), HttpErrorKind::Broker);
    assert_eq!(e.stage(), TransportStage::ResponseReceived);
    assert_eq!(e.attempt(), 1);
    assert_clean(&err);
}
