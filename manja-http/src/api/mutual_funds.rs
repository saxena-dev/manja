//! Mutual funds API group: `/mf/...`
//!
//! This module provides functionality to interact with the Kite Connect
//! Mutual Funds (Coin) HTTP APIs for reading MF orders, SIPs, holdings,
//! and the instruments CSV dump.

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::{Error, Result};
use manja_core::models::{
    KiteApiResponse, MfHolding, MfInstrument, MfOrder, MfOrderId, MfOrderRequest, MfSip,
    MfSipCreateRequest, MfSipId, MfSipModifyRequest,
};
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

    use crate::client::HTTPClient;
    use crate::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
    use crate::credentials::KiteCredentials;
    use crate::error::Error;
    use crate::test_utils::{
        add_mocks, get_http_test_client, APIEndpoint, HTTPMethod, TestResponse,
    };
    use manja_core::error::KiteApiException;
    use manja_core::models::{
        KiteApiResponse, MfHolding, MfInstrument, MfOrder, MfOrderId, MfOrderRequest, MfSip,
        MfSipCreateRequest, MfSipId, MfSipModifyRequest, TransactionType,
    };

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
            ("POST", "/mf/orders"),
            "./kiteconnect-mocks/mf_order_response.json",
        );
        mmap.insert(
            ("DELETE", "/mf/orders/3bb085d1-5038-450e-a807-6543fef6c9ae"),
            "./kiteconnect-mocks/mf_order_cancel.json",
        );
        mmap.insert(
            ("POST", "/mf/sips"),
            "./kiteconnect-mocks/mf_sip_place.json",
        );
        mmap.insert(
            ("PUT", "/mf/sips/986124545877922"),
            "./kiteconnect-mocks/mf_sip_modify.json",
        );
        mmap.insert(
            ("DELETE", "/mf/sips/986124545877922"),
            "./kiteconnect-mocks/mf_sip_cancel.json",
        );
        mmap.insert(
            ("GET", "/mf/sips/181635213661372"),
            "./kiteconnect-mocks/mf_sip_info.json",
        );
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
    async fn test_mf_place_order_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let request = MfOrderRequest {
            tradingsymbol: "INF179K01VY8".to_string(),
            transaction_type: TransactionType::BUY,
            amount: Some(5000.0),
            quantity: None,
            tag: Some("test".to_string()),
        };

        let response = client
            .mutual_funds()
            .place_order(&request)
            .await
            .unwrap();

        let data: MfOrderId = response.data.expect("expected MF order id");
        assert_eq!(
            data.order_id,
            "3bb085d1-5038-450e-a807-6543fef6c9ae"
        );
    }

    #[tokio::test]
    async fn test_mf_cancel_order_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client
            .mutual_funds()
            .cancel_order("3bb085d1-5038-450e-a807-6543fef6c9ae")
            .await
            .unwrap();

        let data: MfOrderId = response.data.expect("expected MF order id");
        assert_eq!(
            data.order_id,
            "3bb085d1-5038-450e-a807-6543fef6c9ae"
        );
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

    #[tokio::test]
    async fn test_mf_place_order_token_exception_error() {
        let mut server = mockito::Server::new_async().await;
        let credentials = KiteCredentials::new(
            "TEST_API_KEY",
            "TEST_API_SECRET",
            "TEST_USER_ID",
            "TEST_PASSWORD",
            "TEST_TOTP",
        );
        let config = Config::from_parts(
            server.url(),
            KITECONNECT_API_LOGIN.to_string(),
            KITECONNECT_API_REDIRECT.to_string(),
            credentials,
        );
        let client = HTTPClient::with_config(config);

        let _m = server
            .mock("POST", "/mf/orders")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Token is invalid or has expired",
                    "error_type": "TokenException"
                }"#,
            )
            .create_async()
            .await;

        let mf_api = MutualFunds::new(&client);
        let request = MfOrderRequest {
            tradingsymbol: "INF179K01VY8".to_string(),
            transaction_type: TransactionType::BUY,
            amount: Some(1000.0),
            quantity: None,
            tag: None,
        };

        let err = mf_api
            .place_order(&request)
            .await
            .expect_err("expected token exception error");

        match err {
            Error::KiteApi(api_err) => {
                assert_eq!(api_err.status_code, 403);
                assert!(matches!(
                    api_err.error_type,
                    KiteApiException::TokenException
                ));
                assert_eq!(api_err.error_type.as_str(), "TokenException");
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_mf_create_sip_token_exception_error() {
        let mut server = mockito::Server::new_async().await;
        let credentials = KiteCredentials::new(
            "TEST_API_KEY",
            "TEST_API_SECRET",
            "TEST_USER_ID",
            "TEST_PASSWORD",
            "TEST_TOTP",
        );
        let config = Config::from_parts(
            server.url(),
            KITECONNECT_API_LOGIN.to_string(),
            KITECONNECT_API_REDIRECT.to_string(),
            credentials,
        );
        let client = HTTPClient::with_config(config);

        let _m = server
            .mock("POST", "/mf/sips")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Token is invalid or has expired",
                    "error_type": "TokenException"
                }"#,
            )
            .create_async()
            .await;

        let mf_api = MutualFunds::new(&client);
        let request = MfSipCreateRequest {
            tradingsymbol: "INF209KB1ZH2".to_string(),
            amount: 5000.0,
            frequency: "weekly".to_string(),
            instalment_day: 0,
            instalments: -1,
            initial_amount: None,
            tag: None,
        };

        let err = mf_api
            .create_sip(&request)
            .await
            .expect_err("expected token exception error");

        match err {
            Error::KiteApi(api_err) => {
                assert_eq!(api_err.status_code, 403);
                assert!(matches!(
                    api_err.error_type,
                    KiteApiException::TokenException
                ));
                assert_eq!(api_err.error_type.as_str(), "TokenException");
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_mf_create_sip_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let request = MfSipCreateRequest {
            tradingsymbol: "INF209KB1ZH2".to_string(),
            amount: 5000.0,
            frequency: "weekly".to_string(),
            instalment_day: 0,
            instalments: -1,
            initial_amount: None,
            tag: Some("coiniossip".to_string()),
        };

        let response = client
            .mutual_funds()
            .create_sip(&request)
            .await
            .unwrap();

        let data: MfSipId = response.data.expect("expected MF sip id");
        assert_eq!(data.sip_id, "986124545877922");
    }

    #[tokio::test]
    async fn test_mf_modify_sip_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let request = MfSipModifyRequest {
            amount: Some(6000.0),
            ..Default::default()
        };

        let response = client
            .mutual_funds()
            .modify_sip("986124545877922", &request)
            .await
            .unwrap();

        let data: MfSipId = response.data.expect("expected MF sip id");
        assert_eq!(data.sip_id, "986124545877922");
    }

    #[tokio::test]
    async fn test_mf_cancel_sip_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client
            .mutual_funds()
            .cancel_sip("986124545877922")
            .await
            .unwrap();

        let data: MfSipId = response.data.expect("expected MF sip id");
        assert_eq!(data.sip_id, "986124545877922");
    }

    #[tokio::test]
    async fn test_mf_sip_single_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client
            .mutual_funds()
            .sip("181635213661372")
            .await
            .unwrap();

        let sip: MfSip = response.data.expect("expected MF sip");
        assert_eq!(sip.sip_id, "181635213661372");
        assert_eq!(sip.tradingsymbol, "INF209KB1ZH2");
        assert_eq!(sip.frequency, "weekly");
    }
}
