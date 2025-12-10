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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mockito::ServerGuard;
    use tokio::join;

    use crate::client::HTTPClient;
    use crate::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
    use crate::credentials::KiteCredentials;
    use crate::error::Error;
    use crate::test_utils::{
        add_mocks, get_http_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
    };
    use manja_core::error::KiteApiException;
    use manja_core::models::{
        BasketMargin, Exchange, OrderCharges, OrderChargesRequest, OrderMargin,
        OrderMarginRequest, OrderType, OrderVariety, ProductType, TransactionType,
    };

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(
            ("POST", "/margins/orders"),
            "./kiteconnect-mocks/order_margins.json",
        );
        mmap.insert(
            ("POST", "/margins/basket?consider_positions=true"),
            "./kiteconnect-mocks/basket_margins.json",
        );
        mmap.insert(
            ("POST", "/charges/orders"),
            "./kiteconnect-mocks/virtual_contract_note.json",
        );
        mmap
    }

    fn sample_order_margin_request() -> OrderMarginRequest {
        OrderMarginRequest {
            exchange: Exchange::NSE,
            tradingsymbol: "INFY".to_string(),
            transaction_type: TransactionType::BUY,
            variety: OrderVariety::Regular,
            product: ProductType::CashAndCarry,
            order_type: OrderType::Market,
            quantity: 1,
            price: 0.0,
            trigger_price: 0.0,
        }
    }

    fn sample_order_charges_requests() -> Vec<OrderChargesRequest> {
        vec![
            OrderChargesRequest {
                order_id: "111111111".to_string(),
                exchange: Exchange::NSE,
                tradingsymbol: "SBIN".to_string(),
                transaction_type: TransactionType::BUY,
                variety: OrderVariety::Regular,
                product: ProductType::CashAndCarry,
                order_type: OrderType::Market,
                quantity: 1,
                average_price: 560.0,
            },
            OrderChargesRequest {
                order_id: "2222222222".to_string(),
                exchange: Exchange::MCX,
                tradingsymbol: "GOLDPETAL24AUGFUT".to_string(),
                transaction_type: TransactionType::SELL,
                variety: OrderVariety::Regular,
                product: ProductType::Normal,
                order_type: OrderType::Limit,
                quantity: 1,
                average_price: 5862.0,
            },
        ]
    }

    #[tokio::test]
    async fn test_margins_basket_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let requests = vec![sample_order_margin_request()];
        let response = client
            .margins()
            .basket(&requests, true)
            .await
            .unwrap();

        let expected =
            read_to_object::<BasketMargin>("./kiteconnect-mocks/basket_margins.json").unwrap();

        let basket = response.data.expect("expected basket margin data");
        assert_eq!(basket.initial.total, expected.initial.total);
        assert_eq!(basket.r#final.total, expected.r#final.total);
    }

    #[tokio::test]
    async fn test_charges_orders_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let requests = sample_order_charges_requests();
        let response = client
            .charges()
            .orders(&requests)
            .await
            .unwrap();

        let expected = read_to_object::<Vec<OrderCharges>>(
            "./kiteconnect-mocks/virtual_contract_note.json",
        )
        .unwrap();

        let charges = response.data.expect("expected order charges data");
        assert_eq!(charges.len(), expected.len());
        assert_eq!(charges[0].tradingsymbol, expected[0].tradingsymbol);
    }

    #[tokio::test]
    async fn test_margins_orders_token_exception_error() {
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
        let mut client = HTTPClient::with_config(config);

        let _m = server
            .mock("POST", "/margins/orders")
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

        let request = sample_order_margin_request();
        let err = client
            .margins()
            .orders(request)
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
}
