//! Portfolio API group: `/portfolio/`
//!
//! This module provides functionality to interact with holdings and positions
//! endpoints of the Kite Connect HTTP API.

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::Result;
use manja_core::models::{Holding, KiteApiResponse, Position};

/// Portfolio-related API endpoints for holdings and positions.
pub struct Portfolio<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Portfolio<'c> {
    /// Creates a new instance of `Portfolio` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Portfolio` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Fetches the list of holdings for the user.
    pub async fn holdings(&self) -> Result<KiteApiResponse<Vec<Holding>>> {
        self.client.get(&"/portfolio/holdings", &self.backoff).await
    }

    /// Fetches the list of positions for the user.
    pub async fn positions(&self) -> Result<KiteApiResponse<Position>> {
        self.client
            .get(&"/portfolio/positions", &self.backoff)
            .await
    }
}

