//! Strict and tolerant (`*_with_rejections`) decoding of the equity order
//! and trade lists, against the loopback harness.
//!
//! Baselines are the official `orders.json`, `order_info.json`,
//! `trades.json` and `order_trades.json`, served unchanged. Each invalid
//! variant is supplemental: one row of `orders.json` or `trades.json` made
//! invalid in one field, with a seeded value that must not reach an error
//! detail or a `Debug` output.

mod support;

use std::sync::Arc;
use std::time::Duration;

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::{Row, Rows};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpErrorKind, ManjaError};
use manja::kite::obs::{InMemoryRecorder, Instrument, Observability};
use manja::kite::protocol::OrderId;
use serde_json::Value;

use support::fixtures;
use support::http::{HttpHarness, Reply};

fn client(base: &str, obs: &Observability) -> HTTPClient {
    let scheduler = SchedulerLimits::default()
        .with_backoff(Duration::from_millis(10), Duration::from_millis(10))
        .unwrap()
        .with_jitter_seed(17);
    let config = Config::new(base).with_limits(HttpLimits::default().with_scheduler(scheduler));
    HTTPClient::with_observability(config, obs.clone())
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

fn recording() -> (Arc<InMemoryRecorder>, Observability) {
    let rec = Arc::new(InMemoryRecorder::new());
    let obs = Observability::with_recorder(rec.clone());
    (rec, obs)
}

async fn serve(body: String) -> HttpHarness {
    HttpHarness::start(vec![Reply::json(body)]).await
}

/// `fixture` with `data[index][field]` replaced by `value`.
fn with_bad_row(fixture: &str, index: usize, field: &str, value: Value) -> String {
    let mut v: Value = serde_json::from_str(&fixtures::json_body(fixture).unwrap()).unwrap();
    v["data"][index][field] = value;
    serde_json::to_string(&v).unwrap()
}

fn decode_detail(err: &ManjaError) -> String {
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Decode, "{err:?}");
    e.detail()
        .map(|d| d.as_str().to_string())
        .unwrap_or_default()
}

/// Rows with one rejection at `index`, decoded rows around it, and the
/// seeded value only in the error and the raw row.
fn assert_one_rejection<T: std::fmt::Debug>(rows: Rows<T>, total: usize, index: usize, seed: &str) {
    assert_eq!(rows.rows.len(), total);
    for (i, row) in rows.rows.iter().enumerate() {
        match row {
            Row::Rejected(e) => {
                assert_eq!(i, index);
                assert_eq!(e.index, index);
                assert!(
                    e.error.contains(seed) || e.raw.to_string().contains(seed),
                    "the detail stays reachable"
                );
                let debug = format!("{e:?}");
                assert!(!debug.contains(seed), "{debug}");
            }
            Row::Decoded(_) => assert_ne!(i, index),
        }
    }
    assert!(!rows.is_complete());
    assert_eq!(rows.items().count(), total - 1);
    let back = rows.into_complete().unwrap_err();
    assert_eq!(back.rows.len(), total, "every row is still there");
}

#[tokio::test]
async fn every_official_list_decodes_the_same_under_both_policies() {
    let (_, obs) = recording();
    let id = OrderId::new("171229000724687").unwrap();
    let tid = OrderId::new("200000000000000").unwrap();

    let h = serve(fixtures::json_body("orders.json").unwrap()).await;
    let c = client(&h.base_url(), &obs);
    let strict = c.orders().list_orders().await.unwrap().data.unwrap();
    let h = serve(fixtures::json_body("orders.json").unwrap()).await;
    let c = client(&h.base_url(), &obs);
    let rows = c
        .orders()
        .list_orders_with_rejections()
        .await
        .unwrap()
        .data
        .unwrap();
    assert!(rows.is_complete());
    assert_eq!(rows.into_complete().unwrap(), strict);
    assert_eq!(strict.len(), 10);

    let h = serve(fixtures::json_body("order_info.json").unwrap()).await;
    let strict = client(&h.base_url(), &obs)
        .orders()
        .get_order_history(&id)
        .await
        .unwrap()
        .data
        .unwrap();
    let h = serve(fixtures::json_body("order_info.json").unwrap()).await;
    let rows = client(&h.base_url(), &obs)
        .orders()
        .get_order_history_with_rejections(&id)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(rows.into_complete().unwrap(), strict);

    let h = serve(fixtures::json_body("trades.json").unwrap()).await;
    let strict = client(&h.base_url(), &obs)
        .orders()
        .list_trades()
        .await
        .unwrap()
        .data
        .unwrap();
    let h = serve(fixtures::json_body("trades.json").unwrap()).await;
    let rows = client(&h.base_url(), &obs)
        .orders()
        .list_trades_with_rejections()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(rows.into_complete().unwrap(), strict);

    let h = serve(fixtures::json_body("order_trades.json").unwrap()).await;
    let strict = client(&h.base_url(), &obs)
        .orders()
        .get_order_trades(&tid)
        .await
        .unwrap()
        .data
        .unwrap();
    let h = serve(fixtures::json_body("order_trades.json").unwrap()).await;
    let rows = client(&h.base_url(), &obs)
        .orders()
        .get_order_trades_with_rejections(&tid)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(rows.into_complete().unwrap(), strict);
}

#[tokio::test]
async fn one_bad_order_is_rejected_in_place_or_fails_the_strict_list() {
    let cases = [
        ("order_id", Value::from("1/../SEEDid")),
        ("order_timestamp", Value::from("SEEDts 2021")),
        ("price", Value::from("SEEDprice")),
    ];
    for (field, value) in cases {
        let seed = value
            .as_str()
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .to_string();
        let body = with_bad_row("orders.json", 3, field, value);
        let (_, obs) = recording();

        let h = serve(body.clone()).await;
        let rows = client(&h.base_url(), &obs)
            .orders()
            .list_orders_with_rejections()
            .await
            .unwrap()
            .data
            .unwrap();
        assert_one_rejection(rows, 10, 3, &seed);

        let h = serve(body).await;
        let err = client(&h.base_url(), &obs)
            .orders()
            .list_orders()
            .await
            .unwrap_err();
        let detail = decode_detail(&err);
        assert_eq!(detail, "1 of 10 rows rejected; first at index 3", "{field}");
        assert!(!format!("{err}").contains(&seed), "{field}");
        assert!(!format!("{err:?}").contains(&seed), "{field}");
    }
}

#[tokio::test]
async fn one_bad_trade_is_rejected_in_place_or_fails_the_strict_list() {
    let cases = [
        ("order_id", Value::from("1/../SEEDid")),
        ("fill_timestamp", Value::from("SEEDts 2021")),
        ("average_price", Value::from("SEEDprice")),
    ];
    for (field, value) in cases {
        let seed = value
            .as_str()
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .to_string();
        let body = with_bad_row("trades.json", 1, field, value);
        let (_, obs) = recording();

        let h = serve(body.clone()).await;
        let rows = client(&h.base_url(), &obs)
            .orders()
            .list_trades_with_rejections()
            .await
            .unwrap()
            .data
            .unwrap();
        assert_one_rejection(rows, 4, 1, &seed);

        let h = serve(body).await;
        let err = client(&h.base_url(), &obs)
            .orders()
            .list_trades()
            .await
            .unwrap_err();
        assert_eq!(
            decode_detail(&err),
            "1 of 4 rows rejected; first at index 1",
            "{field}"
        );
        assert!(!format!("{err}").contains(&seed), "{field}");
        assert!(!format!("{err:?}").contains(&seed), "{field}");
    }
}

// ---- a strict rejection is a failure everywhere ------------------------

#[tokio::test]
async fn a_strict_rejection_is_recorded_as_a_failure() {
    let (rec, obs) = recording();
    let body = with_bad_row("orders.json", 2, "price", Value::from("SEEDprice"));
    let h = serve(body).await;
    let c = client(&h.base_url(), &obs);
    c.orders().list_orders().await.unwrap_err();

    assert_eq!(
        rec.counter(
            Instrument::HttpOperationsTotal,
            &["GET", "/orders", "read", "decode_error"]
        ),
        1
    );
    assert_eq!(
        rec.counter(
            Instrument::HttpAttemptsTotal,
            &["GET", "/orders", "decode_error"]
        ),
        1
    );
    assert_eq!(h.requests().len(), 1, "a decode error is not retried");
    // The span's result is checked in tests/http_order_rows_span.rs.
    let d = c.diagnostics();
    let [f] = d.last_failures.as_slice() else {
        panic!("one recorded failure: {:?}", d.last_failures)
    };
    assert_eq!(f.kind, HttpErrorKind::Decode);
}

// ---- the rejected-row counter ------------------------------------------

#[tokio::test]
async fn rejected_rows_are_counted_once_per_response_under_both_policies() {
    let (rec, obs) = recording();
    let counted = || rec.counter(Instrument::HttpRejectedRowsTotal, &["/orders"]);

    // A complete response records nothing.
    let h = serve(fixtures::json_body("orders.json").unwrap()).await;
    client(&h.base_url(), &obs)
        .orders()
        .list_orders_with_rejections()
        .await
        .unwrap();
    assert_eq!(rec.series_count(Instrument::HttpRejectedRowsTotal), 0);

    // Two bad rows: +2 under the tolerant policy.
    let mut v: Value = serde_json::from_str(&fixtures::json_body("orders.json").unwrap()).unwrap();
    v["data"][1]["price"] = Value::from("x");
    v["data"][6]["order_id"] = Value::from("..");
    let two_bad = serde_json::to_string(&v).unwrap();
    let h = serve(two_bad.clone()).await;
    let rows = client(&h.base_url(), &obs)
        .orders()
        .list_orders_with_rejections()
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(rows.rejected().map(|e| e.index).collect::<Vec<_>>(), [1, 6]);
    assert_eq!(counted(), 2);

    // The same response under the strict policy: +2 again, and the detail
    // names the first index.
    let h = serve(two_bad).await;
    let err = client(&h.base_url(), &obs)
        .orders()
        .list_orders()
        .await
        .unwrap_err();
    assert_eq!(
        decode_detail(&err),
        "2 of 10 rows rejected; first at index 1"
    );
    assert_eq!(counted(), 4);
    // Labelled by endpoint only.
    assert_eq!(rec.series_count(Instrument::HttpRejectedRowsTotal), 1);
    assert_eq!(
        Instrument::HttpRejectedRowsTotal.label_keys().len(),
        1,
        "endpoint only"
    );
}

// ---- the envelope is classified as before --------------------------------

/// Which list method to call.
#[derive(Clone, Copy, Debug)]
enum List {
    Orders,
    History,
    Trades,
    OrderTrades,
}

const LISTS: [List; 4] = [List::Orders, List::History, List::Trades, List::OrderTrades];

/// Call `list` under a policy: the number of rows it returned, or its error.
async fn call(c: &HTTPClient, list: List, tolerant: bool) -> Result<usize, ManjaError> {
    let id = OrderId::new("171229000724687").unwrap();
    let o = c.orders();
    Ok(match (list, tolerant) {
        (List::Orders, false) => o.list_orders().await?.data.unwrap().len(),
        (List::Orders, true) => o
            .list_orders_with_rejections()
            .await?
            .data
            .unwrap()
            .rows
            .len(),
        (List::History, false) => o.get_order_history(&id).await?.data.unwrap().len(),
        (List::History, true) => o
            .get_order_history_with_rejections(&id)
            .await?
            .data
            .unwrap()
            .rows
            .len(),
        (List::Trades, false) => o.list_trades().await?.data.unwrap().len(),
        (List::Trades, true) => o
            .list_trades_with_rejections()
            .await?
            .data
            .unwrap()
            .rows
            .len(),
        (List::OrderTrades, false) => o.get_order_trades(&id).await?.data.unwrap().len(),
        (List::OrderTrades, true) => o
            .get_order_trades_with_rejections(&id)
            .await?
            .data
            .unwrap()
            .rows
            .len(),
    })
}

#[tokio::test]
async fn envelope_failures_are_the_same_on_every_list_and_policy() {
    // Supplemental envelopes. The details are those the single decoding path
    // produced before these lists had a tolerant form.
    let error =
        r#"{"status":"error","message":"Invalid order","error_type":"InputException","data":null}"#;
    let token = r#"{"status":"error","message":"Session expired","error_type":"TokenException","data":null}"#;
    let cases: [(u16, &str, HttpErrorKind, Option<&str>); 9] = [
        (
            200,
            "{not json",
            HttpErrorKind::Decode,
            Some("malformed JSON success body: syntax error at line 1 column 2"),
        ),
        (
            200,
            "[1]",
            HttpErrorKind::Decode,
            Some("the success body is not a JSON object"),
        ),
        (
            200,
            r#"{"data":[]}"#,
            HttpErrorKind::Decode,
            Some("the envelope lacks status = \"success\""),
        ),
        (
            200,
            r#"{"status":"success"}"#,
            HttpErrorKind::Decode,
            Some("the success envelope has no data"),
        ),
        (
            200,
            r#"{"status":"success","data":null}"#,
            HttpErrorKind::Decode,
            Some("the success envelope has no data"),
        ),
        (
            200,
            r#"{"status":"success","data":{"order_id":"1"}}"#,
            HttpErrorKind::Decode,
            Some("the payload does not match the endpoint's type: data error"),
        ),
        (200, error, HttpErrorKind::Broker, None),
        (400, error, HttpErrorKind::Broker, None),
        (403, token, HttpErrorKind::AuthRejected, None),
    ];
    let (_, obs) = recording();
    for (status, body, kind, detail) in cases {
        let mut seen = Vec::new();
        for list in LISTS {
            for tolerant in [false, true] {
                let h = HttpHarness::start(vec![Reply::Respond {
                    status,
                    content_type: "application/json",
                    body: body.as_bytes().to_vec(),
                }])
                .await;
                let err = call(&client(&h.base_url(), &obs), list, tolerant)
                    .await
                    .unwrap_err();
                let e = err.as_http().unwrap();
                assert_eq!(e.kind(), kind, "{list:?} {tolerant} {status} {body}");
                let got = e.detail().map(|d| d.as_str().to_string());
                if let Some(expected) = detail {
                    assert_eq!(got.as_deref(), Some(expected), "{list:?} {tolerant} {body}");
                }
                let broker = e.broker().map(|b| {
                    (
                        b.error_type().map(|t| format!("{t:?}")),
                        b.message().map(|m| m.as_str().to_string()),
                    )
                });
                seen.push((e.http_status(), got, broker));
            }
        }
        assert!(
            seen.windows(2).all(|w| w[0] == w[1]),
            "{status} {body}: {seen:?}"
        );
    }
}

#[tokio::test]
async fn an_array_of_non_objects_is_rows_rejected_not_a_type_mismatch() {
    // Supplemental: data is an array, but of numbers. Each element is
    // decoded on its own, so each is a rejected row: the strict detail is the
    // row count, with no serde text; the tolerant result keeps both rows.
    let body = r#"{"status":"success","data":[1,2]}"#;
    let (_, obs) = recording();
    for list in LISTS {
        let h = serve(body.to_string()).await;
        let err = call(&client(&h.base_url(), &obs), list, false)
            .await
            .unwrap_err();
        assert_eq!(
            decode_detail(&err),
            "2 of 2 rows rejected; first at index 0",
            "{list:?}"
        );
        let h = serve(body.to_string()).await;
        assert_eq!(
            call(&client(&h.base_url(), &obs), list, true)
                .await
                .unwrap(),
            2,
            "{list:?}"
        );
    }
}

#[tokio::test]
async fn one_bad_row_in_a_history_or_order_trades_list_is_handled_the_same_way() {
    // Supplemental, derived from order_info.json and order_trades.json.
    let id = OrderId::new("171229000724687").unwrap();
    let (_, obs) = recording();

    let history = with_bad_row("order_info.json", 1, "price", Value::from("SEEDprice"));
    let total = serde_json::from_str::<Value>(&history).unwrap()["data"]
        .as_array()
        .unwrap()
        .len();
    let h = serve(history.clone()).await;
    let rows = client(&h.base_url(), &obs)
        .orders()
        .get_order_history_with_rejections(&id)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_one_rejection(rows, total, 1, "SEEDprice");
    let h = serve(history).await;
    let err = client(&h.base_url(), &obs)
        .orders()
        .get_order_history(&id)
        .await
        .unwrap_err();
    assert_eq!(
        decode_detail(&err),
        format!("1 of {total} rows rejected; first at index 1")
    );
    assert!(!format!("{err}").contains("SEEDprice"));

    let trades = with_bad_row(
        "order_trades.json",
        0,
        "order_id",
        Value::from("1/../SEEDid"),
    );
    let total = serde_json::from_str::<Value>(&trades).unwrap()["data"]
        .as_array()
        .unwrap()
        .len();
    let h = serve(trades.clone()).await;
    let rows = client(&h.base_url(), &obs)
        .orders()
        .get_order_trades_with_rejections(&id)
        .await
        .unwrap()
        .data
        .unwrap();
    assert_one_rejection(rows, total, 0, "SEEDid");
    let h = serve(trades).await;
    let err = client(&h.base_url(), &obs)
        .orders()
        .get_order_trades(&id)
        .await
        .unwrap_err();
    assert_eq!(
        decode_detail(&err),
        format!("1 of {total} rows rejected; first at index 0")
    );
    assert!(!format!("{err}").contains("SEEDid"));
}
