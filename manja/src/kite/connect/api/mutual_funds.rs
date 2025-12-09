//! Mutual funds API group: `/mf/...`
//!
//! This module mirrors the Mutual Funds HTTP API group provided by the
//! `manja-http` crate, but uses the facade error type and client.

use crate::kite::connect::api::{create_backoff_policy, BackoffPolicy};
use crate::kite::connect::{
    client::HTTPClient,
    models::{KiteApiResponse, MfHolding, MfInstrument, MfOrder, MfSip},
};
use crate::kite::error::Result;

/// Mutual funds APIs for orders, SIPs, holdings, and instruments.
pub struct MutualFunds<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> MutualFunds<'c> {
    /// Creates a new instance of `MutualFunds` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `MutualFunds` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Retrieve all MF orders over the last 7 days.
    pub async fn orders(&self) -> Result<KiteApiResponse<Vec<MfOrder>>> {
        self.client.get("/mf/orders", &self.backoff).await
    }

    /// Retrieve a single MF order by `order_id`.
    pub async fn order(&self, order_id: &str) -> Result<KiteApiResponse<MfOrder>> {
        let path = format!("/mf/orders/{}", order_id);
        self.client.get(&path, &self.backoff).await
    }

    /// Retrieve all active and paused MF SIPs.
    pub async fn sips(&self) -> Result<KiteApiResponse<Vec<MfSip>>> {
        self.client.get("/mf/sips", &self.backoff).await
    }

    /// Retrieve MF holdings available in the DEMAT.
    pub async fn holdings(&self) -> Result<KiteApiResponse<Vec<MfHolding>>> {
        self.client.get("/mf/holdings", &self.backoff).await
    }

    /// Retrieve the raw MF instruments CSV dump.
    pub async fn instruments_csv(&self) -> Result<String> {
        self.client.get_raw("/mf/instruments", &self.backoff).await
    }

    /// Retrieve the parsed MF instruments list.
    ///
    /// This is a thin wrapper around the underlying transport crate’s MF
    /// instruments endpoint and returns the same typed data structures.
    pub async fn instruments(&self) -> Result<Vec<MfInstrument>> {
        self.client.inner_mutual_funds_instruments().await
    }
}

