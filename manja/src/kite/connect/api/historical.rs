//! Historical candle data API group: `/instruments/historical/`
//!
//! This module mirrors the historical HTTP API group provided by the
//! `manja-http` crate, but uses the facade error type and client.

use chrono::NaiveDateTime;

use crate::kite::connect::api::{create_backoff_policy, BackoffPolicy};
use crate::kite::connect::{
    client::HTTPClient,
    models::{HistoricalData, HistoricalInterval, KiteApiResponse},
};
use crate::kite::error::Result;

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
    /// This is a thin wrapper around the underlying transport crate’s
    /// historical API and returns the same typed data structures.
    pub async fn candles(
        &self,
        instrument_token: u32,
        interval: HistoricalInterval,
        from: NaiveDateTime,
        to: NaiveDateTime,
        continuous: bool,
        oi: bool,
    ) -> Result<KiteApiResponse<HistoricalData>> {
        self.client
            .inner_historical_candles(instrument_token, interval, from, to, continuous, oi)
            .await
    }
}

