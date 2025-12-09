//! Market quotes and instruments API group: `/quote/` and `/instruments/`
//!
//! This module provides functionality to fetch market quotes and the instrument
//! master CSV dump from the Kite Connect HTTP API.

use std::collections::HashMap;

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::{Error, Result};
use manja_core::models::{Exchange, Instrument, KiteApiResponse, KiteQuote, QuoteMode};

/// Market quotes APIs and instrument master endpoints.
pub struct Market<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Market<'c> {
    /// Creates a new instance of `Market` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Market` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // Parses the CSV response into a vector of `Instrument`.
    //
    // This function will return an error if the CSV data cannot be parsed.
    fn parse_instruments(data: &str) -> Result<Vec<Instrument>> {
        let mut rdr = csv::Reader::from_reader(data.as_bytes());
        let mut records = Vec::new();

        for result in rdr.deserialize() {
            let record: Instrument =
                result.map_err(|_| Error::Internal("CSV parse error".to_string()))?;
            records.push(record);
        }

        Ok(records)
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Retrieve the CSV dump of all tradable instruments on an exchange.
    ///
    /// The instrument list API returns a gzipped CSV dump of instruments across
    /// all exchanges (if not specified) that can be imported into a database.
    /// The dump is generated once everyday and hence `last_price` is not real time.
    pub async fn get_instruments_csv(&self, exchange: Option<Exchange>) -> Result<String> {
        let path = exchange.map_or("/instruments".to_string(), |x| {
            format!("/instruments/{}", x)
        });
        self.client.get_raw(&path, &self.backoff).await
    }

    /// Retrieve all tradable instruments.
    ///
    /// WARNING: The instrument list API returns large amounts of data. It's
    /// best to request it once a day and cache the instrument data.
    pub async fn get_instruments_all(&self) -> Result<Vec<Instrument>> {
        let instruments = self.get_instruments_csv(None).await?;
        Market::parse_instruments(&instruments)
    }

    /// Retrieve all tradable instruments from a particular exchange.
    pub async fn get_instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>> {
        let instruments = self.get_instruments_csv(Some(exchange)).await?;
        Market::parse_instruments(&instruments)
    }

    /// Retrieve market quotes for one or more instruments.
    ///
    /// The query vector is typically built using [`Instrument::to_query`].
    #[allow(private_bounds)]
    pub async fn get_quotes<Q>(
        &self,
        query: &Vec<(&str, &str)>,
    ) -> Result<KiteApiResponse<HashMap<String, Q>>>
    where
        Q: KiteQuote,
    {
        let (path, limit) = match Q::mode() {
            QuoteMode::Full => ("/quote", std::cmp::min(500, query.len())),
            QuoteMode::OHLC => ("/quote/ohlc", std::cmp::min(1000, query.len())),
            QuoteMode::LTP => ("/quote/ltp", std::cmp::min(1000, query.len())),
        };
        self.client
            .get_with_query(path, &query[..limit], &self.backoff)
            .await
    }
}
