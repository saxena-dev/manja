//! Margin calculation and charges API group: `/margins/` and `/charges/`
//!
//! This module provides functionality to calculate margin requirements and
//! order-wise charges using the Kite Connect HTTP API.

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::Result;
use manja_core::models::{
    BasketMargin, KiteApiResponse, OrderCharges, OrderChargesRequest, OrderMargin,
    OrderMarginRequest,
};

/// Margin calculation APIs let you calculate `span`, `exposure`, `option premium`,
/// `additional`, `bo`, `cash`, `var`, and `pnl` values for a list of orders.
pub struct Margins<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Margins<'c> {
    /// Creates a new instance of `Margins` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Margins` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Calculates margins for each order considering the existing positions
    /// and open orders.
    pub async fn orders(
        &self,
        request: OrderMarginRequest,
    ) -> Result<KiteApiResponse<OrderMargin>> {
        self.client
            .post(&"/margins/orders", request, &self.backoff)
            .await
    }

    /// Calculates margins for spread orders.
    pub async fn basket(
        &self,
        requests: &[OrderMarginRequest],
        consider_positions: bool,
    ) -> Result<KiteApiResponse<BasketMargin>> {
        self.client
            .post(
                &format!("/margins/basket?consider_positions={}", consider_positions),
                requests,
                &self.backoff,
            )
            .await
    }
}

/// A virtual contract provides detailed charges order-wise for brokerage,
/// STT, stamp duty, exchange transaction charges, SEBI turnover charge, and GST.
pub struct Charges<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Charges<'c> {
    /// Creates a new instance of `Charges` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Charges` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Calculates order-wise charges for the order book.
    pub async fn orders(
        &self,
        requests: &[OrderChargesRequest],
    ) -> Result<KiteApiResponse<Vec<OrderCharges>>> {
        self.client
            .post(&"/charges/orders", requests, &self.backoff)
            .await
    }
}

