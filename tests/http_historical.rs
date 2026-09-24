//! Historical candle data against the loopback harness.
//!
//! Baselines are the official `historical_minute.json` and
//! `historical_oi.json`, served unchanged. Request wire expectations are
//! written out from the documented examples (`kite:historical.md:51,118`),
//! and expected candle values are written out by hand from the fixtures.

mod support;

use std::time::Duration;

use chrono::{DateTime, FixedOffset, TimeZone};
use manja::kite::connect::admission::{
    Admission, AdmissionLimits, QuotaProfile, RateClass, Window,
};
use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::{CandleInterval, HistoricalRequest, LTPQuote};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::error::{HttpErrorKind, TransportStage};
use manja::kite::obs::schema::{Endpoint, Method};
use manja::kite::protocol::InstrumentToken;

use support::fixtures;
use support::http::{HttpHarness, RecordedRequest, Reply};

fn client(base: &str) -> HTTPClient {
    let scheduler = SchedulerLimits::default()
        .with_backoff(Duration::from_millis(10), Duration::from_millis(10))
        .unwrap()
        .with_jitter_seed(11);
    let config = Config::new(base).with_limits(HttpLimits::default().with_scheduler(scheduler));
    HTTPClient::with_config(config)
        .unwrap()
        .with_credentials(Credentials::new("test_api_key", "test_access_token").unwrap())
}

async fn serve(fixture: &str) -> HttpHarness {
    HttpHarness::start(vec![Reply::json(fixtures::json_body(fixture).unwrap())]).await
}

fn only(h: &HttpHarness) -> RecordedRequest {
    let [r] = h.requests().try_into().unwrap();
    r
}

fn ist(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(5 * 3600 + 30 * 60)
        .unwrap()
        .with_ymd_and_hms(y, mo, d, h, mi, 0)
        .unwrap()
}

/// The path and the decoded query pairs of a request target.
fn split(target: &str) -> (String, Vec<(String, String)>) {
    fn dec(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'+' => out.push(b' '),
                b'%' => {
                    out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                    i += 2;
                }
                c => out.push(c),
            }
            i += 1;
        }
        String::from_utf8(out).unwrap()
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let pairs = query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap();
            (dec(k), dec(v))
        })
        .collect();
    (path.to_string(), pairs)
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn acc_minutes() -> HistoricalRequest {
    // kite:historical.md:51: NSE-ACC (5633), minute, 2017-12-15 09:15 to 09:20.
    HistoricalRequest::new(
        InstrumentToken::new(5633),
        CandleInterval::Minute,
        ist(2017, 12, 15, 9, 15),
        ist(2017, 12, 15, 9, 20),
    )
}

#[tokio::test]
async fn minute_candles_decode_from_the_official_sample() {
    let h = serve("historical_minute.json").await;
    let data = client(&h.base_url())
        .market()
        .get_historical(&acc_minutes())
        .await
        .unwrap()
        .data
        .unwrap();
    let r = only(&h);
    assert_eq!(r.method, "GET");
    let (path, query) = split(&r.target);
    assert_eq!(path, "/instruments/historical/5633/minute");
    assert_eq!(
        query,
        pairs(&[
            ("from", "2017-12-15 09:15:00"),
            ("to", "2017-12-15 09:20:00")
        ])
    );

    let c = &data.candles;
    assert_eq!(c.len(), 6);
    assert_eq!(c[0].timestamp, ist(2017, 12, 15, 9, 15));
    assert_eq!(
        (c[0].open, c[0].high, c[0].low, c[0].close),
        (1704.5, 1705.0, 1699.25, 1702.8)
    );
    assert_eq!(c[5].timestamp, ist(2017, 12, 15, 9, 20));
    assert_eq!(
        (c[5].open, c[5].high, c[5].low, c[5].close),
        (1699.8, 1700.0, 1696.55, 1696.9)
    );
    let volumes: Vec<u64> = c.iter().map(|k| k.volume).collect();
    assert_eq!(volumes, [2499, 1271, 831, 771, 543, 802]);
    assert!(c.iter().all(|k| k.oi.is_none()));
    let minutes: Vec<_> = c.iter().map(|k| k.timestamp).collect();
    assert_eq!(
        minutes,
        (15..=20)
            .map(|m| ist(2017, 12, 15, 9, m))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn open_interest_is_requested_and_decoded() {
    let h = serve("historical_oi.json").await;
    // kite:historical.md:118: NIFTY19DECFUT (12517890), minute, with OI.
    let mut request = HistoricalRequest::new(
        InstrumentToken::new(12517890),
        CandleInterval::Minute,
        ist(2019, 12, 4, 9, 15),
        ist(2019, 12, 4, 9, 20),
    );
    request.oi = true;
    let data = client(&h.base_url())
        .market()
        .get_historical(&request)
        .await
        .unwrap()
        .data
        .unwrap();
    let (path, query) = split(&only(&h).target);
    assert_eq!(path, "/instruments/historical/12517890/minute");
    assert_eq!(
        query,
        pairs(&[
            ("from", "2019-12-04 09:15:00"),
            ("to", "2019-12-04 09:20:00"),
            ("oi", "1"),
        ])
    );
    let c = &data.candles;
    assert_eq!(c.len(), 6);
    assert_eq!(
        (c[0].open, c[0].high, c[0].low, c[0].close, c[0].volume),
        (12009.9, 12019.35, 12001.25, 12001.5, 163275)
    );
    let oi: Vec<Option<u64>> = c.iter().map(|k| k.oi).collect();
    assert_eq!(
        oi,
        [13667775, 13667775, 13758000, 13758000, 13758000, 13777050].map(Some)
    );
}

#[tokio::test]
async fn continuous_day_candles_send_the_flag_and_convert_the_range_to_ist() {
    let h = serve("historical_minute.json").await;
    let utc = FixedOffset::east_opt(0).unwrap();
    let mut request = HistoricalRequest::new(
        InstrumentToken::new(5633),
        CandleInterval::Day,
        // 2017-12-01 00:00 UTC is 05:30 IST.
        utc.with_ymd_and_hms(2017, 12, 1, 0, 0, 0).unwrap(),
        utc.with_ymd_and_hms(2017, 12, 15, 0, 0, 0).unwrap(),
    );
    request.continuous = true;
    client(&h.base_url())
        .market()
        .get_historical(&request)
        .await
        .unwrap();
    let (path, query) = split(&only(&h).target);
    assert_eq!(path, "/instruments/historical/5633/day");
    assert_eq!(
        query,
        pairs(&[
            ("from", "2017-12-01 05:30:00"),
            ("to", "2017-12-15 05:30:00"),
            ("continuous", "1"),
        ])
    );
}

#[tokio::test]
async fn every_documented_interval_is_sent_by_its_wire_name() {
    // kite:historical.md:14.
    let intervals = [
        (CandleInterval::Minute, "minute"),
        (CandleInterval::ThreeMinute, "3minute"),
        (CandleInterval::FiveMinute, "5minute"),
        (CandleInterval::TenMinute, "10minute"),
        (CandleInterval::FifteenMinute, "15minute"),
        (CandleInterval::ThirtyMinute, "30minute"),
        (CandleInterval::SixtyMinute, "60minute"),
        (CandleInterval::Day, "day"),
    ];
    let body = fixtures::json_body("historical_minute.json").unwrap();
    let h = HttpHarness::start(
        intervals
            .iter()
            .map(|_| Reply::json(body.clone()))
            .collect(),
    )
    .await;
    // The documented 3 per second would hold back every fourth request; a
    // raised historical window keeps this test about the wire only.
    let admission = Admission::new(
        QuotaProfile::kite_v3()
            .with_version("wire-test")
            .with_windows(
                RateClass::Historical,
                vec![Window::new(100, Duration::from_secs(1))],
            )
            .unwrap(),
        AdmissionLimits::default(),
    );
    let c = HTTPClient::builder(Config::new(&*h.base_url()))
        .admission(admission)
        .credentials(Credentials::new("k", "t").unwrap())
        .build()
        .unwrap();
    for (interval, _) in intervals {
        let mut request = acc_minutes();
        request.interval = interval;
        c.market().get_historical(&request).await.unwrap();
    }
    let paths: Vec<String> = h.requests().iter().map(|r| split(&r.target).0).collect();
    let expected: Vec<String> = intervals
        .iter()
        .map(|(_, wire)| format!("/instruments/historical/5633/{wire}"))
        .collect();
    assert_eq!(paths, expected);
}

#[tokio::test]
async fn a_range_that_ends_before_it_starts_sends_nothing() {
    let h = HttpHarness::start(vec![]).await;
    let mut request = acc_minutes();
    request.to = ist(2017, 12, 15, 9, 0);
    let err = client(&h.base_url())
        .market()
        .get_historical(&request)
        .await
        .unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Validation);
    assert_eq!(e.stage(), TransportStage::NotStarted);
    assert_eq!(e.endpoint(), Endpoint::InstrumentsHistorical);
    assert!(h.requests().is_empty());
}

#[tokio::test]
async fn a_malformed_candle_is_a_decode_error() {
    // Supplemental, derived from historical_minute.json: the first candle
    // loses its volume.
    let body = fixtures::json_body("historical_minute.json")
        .unwrap()
        .replacen("1702.8,\n        2499\n", "1702.8\n", 1);
    assert_ne!(body, fixtures::json_body("historical_minute.json").unwrap());
    let h = HttpHarness::start(vec![Reply::json(body)]).await;
    let err = client(&h.base_url())
        .market()
        .get_historical(&acc_minutes())
        .await
        .unwrap_err();
    assert_eq!(err.as_http().unwrap().kind(), HttpErrorKind::Decode);
}

#[tokio::test]
async fn a_transient_failure_is_retried() {
    // Supplemental: a 503 before the official sample.
    let h = HttpHarness::start(vec![
        Reply::Respond {
            status: 503,
            content_type: "text/html",
            body: b"<html>unavailable</html>".to_vec(),
        },
        Reply::json(fixtures::json_body("historical_minute.json").unwrap()),
    ])
    .await;
    let data = client(&h.base_url())
        .market()
        .get_historical(&acc_minutes())
        .await
        .unwrap()
        .data
        .unwrap();
    assert_eq!(data.candles.len(), 6);
    assert_eq!(h.requests().len(), 2);
}

#[test]
fn historical_data_has_its_own_quota_class_and_endpoint_label() {
    assert_eq!(
        RateClass::of(Method::Get, Endpoint::InstrumentsHistorical),
        RateClass::Historical
    );
    assert_eq!(
        Endpoint::InstrumentsHistorical.as_str(),
        "/instruments/historical/{instrument_token}/{interval}"
    );
}

#[tokio::test]
async fn the_client_admits_historical_requests_in_their_own_class() {
    // Supplemental replies: three candle responses and one quote. With a
    // 1 ms admission wait, a request is refused rather than delayed when its
    // class has no capacity left in the current second.
    let h = HttpHarness::start(vec![
        Reply::json(fixtures::json_body("historical_minute.json").unwrap()),
        Reply::json(fixtures::json_body("historical_minute.json").unwrap()),
        Reply::json(fixtures::json_body("historical_minute.json").unwrap()),
        Reply::json(fixtures::json_body("ltp.json").unwrap()),
    ])
    .await;
    let admission = Admission::new(
        QuotaProfile::kite_v3(),
        AdmissionLimits::default()
            .with_wait(Duration::from_millis(1))
            .unwrap(),
    );
    let c = HTTPClient::builder(Config::new(&*h.base_url()))
        .admission(admission)
        .credentials(Credentials::new("k", "t").unwrap())
        .build()
        .unwrap();
    // Three in the same second: more than the 1 per second of the smallest
    // (unclassified) rate, so the class is not the fallback.
    for _ in 0..3 {
        c.market().get_historical(&acc_minutes()).await.unwrap();
    }
    // The fourth exceeds 3 per second, so the class is not Standard's 10.
    let err = c.market().get_historical(&acc_minutes()).await.unwrap_err();
    let e = err.as_http().unwrap();
    assert_eq!(e.kind(), HttpErrorKind::Admission);
    assert_eq!(e.stage(), TransportStage::NotStarted);
    assert_eq!(e.endpoint(), Endpoint::InstrumentsHistorical);
    // A quote in the same second is admitted: the classes do not share a window.
    c.market()
        .get_quotes::<LTPQuote>(&["NSE:INFY"])
        .await
        .unwrap();
    assert_eq!(h.requests().len(), 4, "the refused request was not sent");
}
