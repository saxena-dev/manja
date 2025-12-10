//! Historical candle data API group: `/instruments/historical/`
//!
//! This module provides functionality to fetch historical OHLCV(+OI) candles
//! for a given instrument token and interval from the Kite Connect HTTP API.

use chrono::NaiveDateTime;
use serde::Serialize;

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::Result;
use manja_core::models::{HistoricalData, HistoricalInterval, KiteApiResponse};

/// Query parameters for the historical candles endpoint.
#[derive(Serialize)]
struct HistoricalQuery<'a> {
    from: &'a str,
    to: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    continuous: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oi: Option<u8>,
}

/// Historical OHLCV(+OI) candles API.
pub struct Historical<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Historical<'c> {
    /// Creates a new instance of `Historical` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Historical` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Retrieve historical candles for an instrument.
    ///
    /// The `from` and `to` parameters are specified in exchange-local time and
    /// are formatted as `yyyy-mm-dd hh:mm:ss` in the underlying HTTP request.
    ///
    /// When `continuous` is `true`, continuous data is requested (where
    /// supported by Kite). When `oi` is `true`, the API returns OI candles
    /// where available and the `oi` field on [`HistoricalData`] entries will
    /// be `Some(value)`.
    pub async fn candles(
        &self,
        instrument_token: u32,
        interval: HistoricalInterval,
        from: NaiveDateTime,
        to: NaiveDateTime,
        continuous: bool,
        oi: bool,
    ) -> Result<KiteApiResponse<HistoricalData>> {
        let path = format!(
            "/instruments/historical/{}/{}",
            instrument_token,
            interval.as_str()
        );

        let from_str = from.format("%Y-%m-%d %H:%M:%S").to_string();
        let to_str = to.format("%Y-%m-%d %H:%M:%S").to_string();

        let query = HistoricalQuery {
            from: &from_str,
            to: &to_str,
            continuous: if continuous { Some(1) } else { None },
            oi: if oi { Some(1) } else { None },
        };

        self.client
            .get_with_query(&path, &query, &self.backoff)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::NaiveDate;
    use tokio::join;

    use crate::test_utils::{
        add_mocks, get_http_test_client, APIEndpoint, HTTPMethod, TestResponse,
    };
    use manja_core::models::HistoricalInterval;

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(
            (
                "GET",
                "/instruments/historical/5633/minute?from=2017-12-15+09%3A15%3A00&to=2017-12-15+09%3A20%3A00",
            ),
            "./kiteconnect-mocks/historical_minute.json",
        );
        mmap.insert(
            (
                "GET",
                "/instruments/historical/12517890/minute?from=2019-12-04+09%3A15%3A00&to=2019-12-04+09%3A20%3A00&oi=1",
            ),
            "./kiteconnect-mocks/historical_oi.json",
        );
        mmap
    }

    #[tokio::test]
    async fn test_historical_candles_without_oi() {
        let (server, client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let from = NaiveDate::from_ymd_opt(2017, 12, 15)
            .unwrap()
            .and_hms_opt(9, 15, 0)
            .unwrap();
        let to = NaiveDate::from_ymd_opt(2017, 12, 15)
            .unwrap()
            .and_hms_opt(9, 20, 0)
            .unwrap();

        let response = Historical::new(&client)
            .candles(5633, HistoricalInterval::Minute, from, to, false, false)
            .await
            .unwrap();

        let candles = response.data.expect("expected candles data").candles;
        assert_eq!(candles.len(), 6);
        assert!((candles[0].open - 1704.5).abs() < f64::EPSILON);
        assert_eq!(candles[0].oi, None);
    }

    #[tokio::test]
    async fn test_historical_candles_with_oi() {
        let (server, client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let from = NaiveDate::from_ymd_opt(2019, 12, 4)
            .unwrap()
            .and_hms_opt(9, 15, 0)
            .unwrap();
        let to = NaiveDate::from_ymd_opt(2019, 12, 4)
            .unwrap()
            .and_hms_opt(9, 20, 0)
            .unwrap();

        let response = Historical::new(&client)
            .candles(12517890, HistoricalInterval::Minute, from, to, false, true)
            .await
            .unwrap();

        let candles = response.data.expect("expected candles data").candles;
        assert_eq!(candles.len(), 6);
        assert!(candles[0].oi.is_some());
        assert_eq!(candles[0].oi, Some(13667775));
    }
}
