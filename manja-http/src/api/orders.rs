//! Orders API group: `/orders/`
//!
//! This module provides functionality to interact with the orders-related
//! endpoints of the Kite Connect HTTP API.

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::Result;
use manja_core::models::{KiteApiResponse, Order, OrderReceipt, Trade};

/// The order APIs let you place orders of different varieties, modify and
/// cancel pending orders, retrieve the daily order book and more.
pub struct Orders<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Orders<'c> {
    /// Creates a new instance of `Orders` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Orders` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Places an order of a particular variety.
    pub async fn place_order(&self, order: &Order) -> Result<KiteApiResponse<OrderReceipt>> {
        self.client
            .post(&format!("/orders/{}", order.variety), order, &self.backoff)
            .await
    }

    /// Modifies an open or pending order.
    pub async fn modify_order(
        &self,
        variety: &str,
        order_id: &str,
        order: &Order,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.client
            .put(
                &format!("/orders/{}/{}", variety, order_id),
                order,
                &self.backoff,
            )
            .await
    }

    /// Cancels an open or pending order.
    pub async fn cancel_order(
        &self,
        variety: &str,
        order_id: &str,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.client
            .delete(
                &format!("/orders/{}/{}", variety, order_id),
                false,
                &self.backoff,
            )
            .await
    }

    /// Retrieves the list of all orders for the day.
    pub async fn orders(&self) -> Result<KiteApiResponse<Vec<Order>>> {
        self.client.get(&"/orders", &self.backoff).await
    }

    /// Retrieves the details of a specific order.
    pub async fn order_history(&self, order_id: &str) -> Result<KiteApiResponse<Vec<Order>>> {
        self.client
            .get(&format!("/orders/{}", order_id), &self.backoff)
            .await
    }

    /// Retrieves the list of all trades for the day.
    pub async fn trades(&self) -> Result<KiteApiResponse<Vec<Trade>>> {
        self.client.get(&"/trades", &self.backoff).await
    }

    /// Retrieves the list of trades for a specific order.
    pub async fn order_trades(&self, order_id: &str) -> Result<KiteApiResponse<Vec<Trade>>> {
        self.client
            .get(&format!("/orders/{}/trades", order_id), &self.backoff)
            .await
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
        add_mocks, get_http_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
    };
    use manja_core::error::KiteApiException;
    use manja_core::models::{Order, Trade};

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(("GET", "/orders"), "./kiteconnect-mocks/orders.json");
        mmap.insert(
            ("GET", "/orders/171229000724687"),
            "./kiteconnect-mocks/order_info.json",
        );
        mmap.insert(("GET", "/trades"), "./kiteconnect-mocks/trades.json");
        mmap.insert(
            ("GET", "/orders/171229000724687/trades"),
            "./kiteconnect-mocks/order_trades.json",
        );
        mmap
    }

    #[tokio::test]
    async fn test_orders_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client.orders().orders().await.unwrap();
        let expected =
            read_to_object::<Vec<Order>>("./kiteconnect-mocks/orders.json").unwrap();

        let orders = response.data.expect("expected orders data");
        assert_eq!(orders.len(), expected.len());
        assert_eq!(orders[0].order_id, expected[0].order_id);
    }

    #[tokio::test]
    async fn test_order_history_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client
            .orders()
            .order_history("171229000724687")
            .await
            .unwrap();
        let expected =
            read_to_object::<Vec<Order>>("./kiteconnect-mocks/order_info.json").unwrap();

        let history = response.data.expect("expected order history");
        assert_eq!(history.len(), expected.len());
        assert_eq!(history[0].order_id, expected[0].order_id);
    }

    #[tokio::test]
    async fn test_trades_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client.orders().trades().await.unwrap();
        let expected =
            read_to_object::<Vec<Trade>>("./kiteconnect-mocks/trades.json").unwrap();

        let trades = response.data.expect("expected trades data");
        assert_eq!(trades.len(), expected.len());
        assert_eq!(trades[0].trade_id, expected[0].trade_id);
    }

    #[tokio::test]
    async fn test_order_trades_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client
            .orders()
            .order_trades("171229000724687")
            .await
            .unwrap();
        let expected =
            read_to_object::<Vec<Trade>>("./kiteconnect-mocks/order_trades.json").unwrap();

        let trades = response.data.expect("expected trades data");
        assert_eq!(trades.len(), expected.len());
        assert_eq!(trades[0].trade_id, expected[0].trade_id);
    }

    #[tokio::test]
    async fn test_orders_token_exception_error() {
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
            .mock("GET", "/orders")
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

        let err = client
            .orders()
            .orders()
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
