//! Mutual funds API group: `/mf/...`
//!
//! This module provides functionality to interact with the Kite Connect
//! Mutual Funds (Coin) HTTP APIs for reading MF orders, SIPs, holdings,
//! and the instruments CSV dump.

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::{Error, Result};
use manja_core::models::{KiteApiResponse, MfHolding, MfInstrument, MfOrder, MfSip};
use serde::Deserialize;

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

    // Parses the CSV response into a vector of `MfInstrument`.
    //
    // This function will return an error if the CSV data cannot be parsed.
    fn parse_instruments(data: &str) -> Result<Vec<MfInstrument>> {
        let mut rdr = csv::Reader::from_reader(data.as_bytes());
        let mut records = Vec::new();

        for result in rdr.deserialize() {
            let record: MfInstrument =
                result.map_err(|_| Error::Internal("CSV parse error".to_string()))?;
            records.push(record);
        }

        Ok(records)
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
    ///
    /// This method supports both the standard Kite response wrapper
    /// (`KiteApiResponse<Vec<MfSip>>`) and a legacy/fixture shape that
    /// only includes a top-level `data` field.
    pub async fn sips(&self) -> Result<KiteApiResponse<Vec<MfSip>>> {
        let raw = self
            .client
            .get_raw("/mf/sips", &self.backoff)
            .await?;

        // First, try the standard `KiteApiResponse` shape.
        if let Ok(resp) = serde_json::from_str::<KiteApiResponse<Vec<MfSip>>>(&raw) {
            return Ok(resp);
        }

        // Fallback to a legacy shape with only `data: Vec<MfSip>`.
        #[derive(Deserialize)]
        struct LegacyMfSipResponse {
            data: Vec<MfSip>,
        }

        let legacy: LegacyMfSipResponse = serde_json::from_str(&raw)?;
        Ok(KiteApiResponse {
            status: "success".to_string(),
            data: Some(legacy.data),
            message: None,
            error_type: None,
        })
    }

    /// Retrieve MF holdings available in the DEMAT.
    pub async fn holdings(&self) -> Result<KiteApiResponse<Vec<MfHolding>>> {
        self.client.get("/mf/holdings", &self.backoff).await
    }

    /// Retrieve the raw MF instruments CSV dump.
    pub async fn instruments_csv(&self) -> Result<String> {
        self.client
            .get_raw("/mf/instruments", &self.backoff)
            .await
    }

    /// Retrieve the parsed MF instruments list.
    ///
    /// WARNING: The MF instrument list API returns a sizeable CSV dump. It's
    /// best to request it once a day and cache the instrument data.
    pub async fn instruments(&self) -> Result<Vec<MfInstrument>> {
        let instruments_csv = self.instruments_csv().await?;
        Self::parse_instruments(&instruments_csv)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::join;

    use crate::test_utils::{
        add_mocks, get_http_test_client, APIEndpoint, HTTPMethod, TestResponse,
    };
    use manja_core::models::{KiteApiResponse, MfHolding, MfInstrument, MfOrder};

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();

        mmap.insert(("GET", "/mf/orders"), "./kiteconnect-mocks/mf_orders.json");
        mmap.insert(
            ("GET", "/mf/orders/2b6ad4b7-c84e-4c76-b459-f3a8994184f1"),
            "./kiteconnect-mocks/mf_orders_info.json",
        );
        mmap.insert(("GET", "/mf/sips"), "./kiteconnect-mocks/mf_sips.json");
        mmap.insert(
            ("GET", "/mf/holdings"),
            "./kiteconnect-mocks/mf_holdings.json",
        );
        mmap.insert(
            ("GET", "/mf/instruments"),
            "./kiteconnect-mocks/mf_instruments.csv",
        );

        mmap
    }

    #[tokio::test]
    async fn test_mf_orders_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client.mutual_funds().orders().await.unwrap();
        let orders = response.data.expect("expected MF orders data");
        assert!(!orders.is_empty());

        let first: &MfOrder = &orders[0];
        assert_eq!(
            first.order_id,
            "271989e0-a64e-4cf3-b4e4-afb8f38dd203"
        );
        assert_eq!(first.tradingsymbol, "INF179K01VY8");
    }

    #[tokio::test]
    async fn test_mf_order_single_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client
            .mutual_funds()
            .order("2b6ad4b7-c84e-4c76-b459-f3a8994184f1")
            .await
            .unwrap();

        let order = response.data.expect("expected MF order data");
        assert_eq!(order.tradingsymbol, "INF761K01EE1");
        assert_eq!(order.amount, 5000.0);
    }

    #[tokio::test]
    async fn test_mf_holdings_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client.mutual_funds().holdings().await.unwrap();
        let holdings = response.data.expect("expected MF holdings data");
        assert!(!holdings.is_empty());

        let first: &MfHolding = &holdings[0];
        assert_eq!(first.tradingsymbol, "INF205K01NT8");
        assert_eq!(first.fund, "INVESCO INDIA TAX PLAN - DIRECT PLAN");
    }

    #[tokio::test]
    async fn test_mf_sips_success_with_legacy_wrapper() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client.mutual_funds().sips().await.unwrap();
        let sips = response.data.expect("expected MF sips data");

        assert!(!sips.is_empty());
        let first = &sips[0];
        assert_eq!(first.tradingsymbol, "INF209K01VD7");
        assert_eq!(first.frequency, "weekly");
    }

    #[tokio::test]
    async fn test_mf_instruments_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let instruments: Vec<MfInstrument> = client
            .mutual_funds()
            .instruments()
            .await
            .unwrap();

        assert!(!instruments.is_empty());
        let first: &MfInstrument = &instruments[0];
        assert_eq!(first.tradingsymbol, "INF209K01157");
        assert_eq!(
            first.name,
            "Aditya Birla Sun Life Advantage Fund"
        );
    }
}
