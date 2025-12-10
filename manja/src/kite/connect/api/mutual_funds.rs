//! Mutual funds API group: `/mf/...`
//!
//! This module mirrors the Mutual Funds HTTP API group provided by the
//! `manja-http` crate, but uses the facade error type and client.

use crate::kite::connect::api::{create_backoff_policy, BackoffPolicy};
use crate::kite::connect::{
    client::HTTPClient,
    models::{
        KiteApiResponse, MfHolding, MfInstrument, MfOrder, MfOrderId, MfOrderRequest, MfSip,
        MfSipCreateRequest, MfSipId, MfSipModifyRequest,
    },
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

    /// Place a new mutual fund order (BUY or SELL).
    pub async fn place_order(
        &self,
        req: &MfOrderRequest,
    ) -> Result<KiteApiResponse<MfOrderId>> {
        self.client
            .post_form("/mf/orders", req, &self.backoff)
            .await
    }

    /// Cancel an open or pending mutual fund order.
    pub async fn cancel_order(
        &self,
        order_id: &str,
    ) -> Result<KiteApiResponse<MfOrderId>> {
        let path = format!("/mf/orders/{}", order_id);
        self.client.delete(&path, false, &self.backoff).await
    }

    /// Retrieve all active and paused MF SIPs.
    pub async fn sips(&self) -> Result<KiteApiResponse<Vec<MfSip>>> {
        self.client.get("/mf/sips", &self.backoff).await
    }

    /// Create a new mutual fund SIP.
    pub async fn create_sip(
        &self,
        req: &MfSipCreateRequest,
    ) -> Result<KiteApiResponse<MfSipId>> {
        self.client
            .post_form("/mf/sips", req, &self.backoff)
            .await
    }

    /// Modify an existing mutual fund SIP.
    pub async fn modify_sip(
        &self,
        sip_id: &str,
        req: &MfSipModifyRequest,
    ) -> Result<KiteApiResponse<MfSipId>> {
        let path = format!("/mf/sips/{}", sip_id);
        self.client.put(&path, req, &self.backoff).await
    }

    /// Cancel an existing mutual fund SIP.
    pub async fn cancel_sip(
        &self,
        sip_id: &str,
    ) -> Result<KiteApiResponse<MfSipId>> {
        let path = format!("/mf/sips/{}", sip_id);
        self.client.delete(&path, false, &self.backoff).await
    }

    /// Retrieve the configuration for a single mutual fund SIP.
    pub async fn sip(&self, sip_id: &str) -> Result<KiteApiResponse<MfSip>> {
        let path = format!("/mf/sips/{}", sip_id);
        self.client.get(&path, &self.backoff).await
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
