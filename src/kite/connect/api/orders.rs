//! Orders API group: `/orders/`
//!
//! This module provides functionality to interact with the orders-related
//! endpoints of Kite Connect API.
//!
//! Placing an order implies registering it with the OMS via the API. This does
//! not guarantee the order's receipt at the exchange. The fate of an order is
//! dependent on several factors including market hours, availability of funds,
//! risk checks and so on. Under normal circumstances, order placement, receipt
//! by the OMS, transport to the exchange, execution, and the confirmation
//! roundtrip happen instantly.
//!
//! When an order is successfully placed, the API returns an `order_id`. The status
//! of the order is not known at the moment of placing because of the aforementioned
//! reasons.
//!
//! Refer to the official [API documentation](https://kite.trade/docs/connect/v3/orders/).
//!
use crate::kite::connect::api::{create_backoff_policy, BackoffPolicy};
use crate::kite::connect::{
    client::HTTPClient,
    models::{KiteApiResponse, Order, OrderReceipt, Trade},
};
use crate::kite::error::Result;

/// The order APIs let you place orders of different varities, modify and
/// cancel pending orders, retrieve the daily order and more.
///
pub struct Orders<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Orders<'c> {
    /// Creates a new instance of `Orders` with default API rate limits.
    ///
    /// # Arguments
    ///
    /// * `client` - A reference to the `HTTPClient` used for making API requests.
    ///
    /// # Returns
    ///
    /// A new instance of `Orders`.
    ///
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Orders` instance.
    ///
    /// # Arguments
    ///
    /// * `backoff` - A `BackoffPolicy` instance specifying the backoff policy.
    ///
    /// # Returns
    ///
    /// The `Orders` instance with the updated backoff policy.
    ///
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Places an order of a particular variety.
    ///
    /// Placing an order implies registering it with the OMS via the API. This does
    /// not guarantee the order's receipt at the exchange. The fate of an order is
    /// dependent on several factors including market hours, availability of funds,
    /// risk checks and so on. Under normal circumstances, order placement, receipt
    /// by the OMS, transport to the exchange, execution, and the confirmation
    /// roundtrip happen instantly.
    ///
    /// When an order is successfully placed, the API returns an `order_id`. The status
    /// of the order is not known at the moment of placing because of the aforementioned
    /// reasons.
    ///
    /// # Arguments
    ///
    /// * `order` - A reference to an `Order` instance containing the order details.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with an `OrderReceipt` on success.
    ///
    pub async fn place_order(&self, order: &Order) -> Result<KiteApiResponse<OrderReceipt>> {
        self.client
            .post(&format!("/orders/{}", order.variety), order, &self.backoff)
            .await
    }

    /// Modifies an open or pending order.
    ///
    /// As long as on order is open or pending in the system, certain attributes of
    /// it may be modified.
    ///
    /// # Arguments
    ///
    /// * `variety` - The variety of the order (e.g., "regular", "amo").
    /// * `order_id` - The unique ID of the order to be modified.
    /// * `order` - A reference to an `Order` instance containing the modified order details.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with an `OrderReceipt` on success.
    ///
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
    ///
    /// As long as on order is open or pending in the system, it can be cancelled.
    ///
    /// # Arguments
    ///
    /// * `variety` - The variety of the order (e.g., "regular", "amo").
    /// * `order_id` - The unique ID of the order to be canceled.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with an `OrderReceipt` on success.
    ///
    pub async fn cancel_order(
        &self,
        variety: &str,
        order_id: &str,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.client
            .delete(
                &format!("/orders/{}/{}", variety, order_id),
                true,
                &self.backoff,
            )
            .await
    }

    /// Retrieves the list of all orders (open and executed) for the day.
    ///
    /// The order history or the order book is transient as it only lives for a day
    /// in the system. When you retrieve orders, you get all the orders for the day
    /// including open, pending, and executed ones.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with a vector of `Order` instances on success.
    ///
    pub async fn list_orders(&self) -> Result<KiteApiResponse<Vec<Order>>> {
        self.client.get(&format!("/orders"), &self.backoff).await
    }

    /// Retrieves the history of a given order.
    ///
    /// # Arguments
    ///
    /// * `order_id` - The unique ID of the order whose history is to be retrieved.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with a vector of `Order` instances on success.
    ///
    pub async fn get_order_history(&self, order_id: &str) -> Result<KiteApiResponse<Vec<Order>>> {
        self.client
            .get(&format!("/orders/{}", order_id), &self.backoff)
            .await
    }

    /// Retrieves the list of all executed trades for the day.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with a vector of `Trade` instances on success.
    ///
    pub async fn list_trades(&self) -> Result<KiteApiResponse<Vec<Trade>>> {
        self.client.get(&format!("/trades"), &self.backoff).await
    }

    /// Retrieves the trades generated by a particular order.
    ///
    /// # Arguments
    ///
    /// * `order_id` - The unique ID of the order whose trades are to be retrieved.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with a vector of `Trade` instances on success.
    ///
    pub async fn get_order_trades(&self, order_id: &str) -> Result<KiteApiResponse<Vec<Trade>>> {
        self.client
            .get(&format!("/orders/{}/trades", order_id), &self.backoff)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mockito::ServerGuard;
    use tokio::join;

    use crate::kite::connect::client::test_utils::{
        add_mocks, get_manja_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
    };
    use crate::kite::connect::client::HTTPClient;
    use crate::kite::connect::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
    use crate::kite::connect::credentials::KiteCredentials;
    use crate::kite::connect::models::{Order, Trade};
    use crate::kite::error::{KiteApiException, ManjaError};
    use crate::test_support::init_tracing;

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
    async fn test_list_orders_success() {
        init_tracing();
        let (server, mut manja_client) = get_manja_test_client().await;
        let server_ptr: *const ServerGuard = &server;
        log::debug!("Server @address: {:p}", server_ptr);
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = manja_client.orders().list_orders().await.unwrap();
        let expected =
            read_to_object::<Vec<Order>>("./kiteconnect-mocks/orders.json").unwrap();

        let orders = response.data.expect("expected orders data");
        assert_eq!(orders.len(), expected.len());
        assert_eq!(orders[0].order_id, expected[0].order_id);
    }

    #[tokio::test]
    async fn test_get_order_history_success() {
        let (server, mut manja_client) = get_manja_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = manja_client
            .orders()
            .get_order_history("171229000724687")
            .await
            .unwrap();
        let expected =
            read_to_object::<Vec<Order>>("./kiteconnect-mocks/order_info.json").unwrap();

        let history = response.data.expect("expected order history");
        assert_eq!(history.len(), expected.len());
        assert_eq!(history[0].order_id, expected[0].order_id);
    }

    #[tokio::test]
    async fn test_list_trades_success() {
        let (server, mut manja_client) = get_manja_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = manja_client.orders().list_trades().await.unwrap();
        let expected =
            read_to_object::<Vec<Trade>>("./kiteconnect-mocks/trades.json").unwrap();

        let trades = response.data.expect("expected trades data");
        assert_eq!(trades.len(), expected.len());
        assert_eq!(trades[0].trade_id, expected[0].trade_id);
    }

    #[tokio::test]
    async fn test_get_order_trades_success() {
        let (server, mut manja_client) = get_manja_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = manja_client
            .orders()
            .get_order_trades("171229000724687")
            .await
            .unwrap();
        let expected =
            read_to_object::<Vec<Trade>>("./kiteconnect-mocks/order_trades.json").unwrap();

        let trades = response.data.expect("expected trades data");
        assert_eq!(trades.len(), expected.len());
        assert_eq!(trades[0].trade_id, expected[0].trade_id);
    }

    #[tokio::test]
    async fn test_list_orders_token_exception_error() {
        init_tracing();
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
            .list_orders()
            .await
            .expect_err("expected token exception error");

        match err {
            ManjaError::KiteApiError(api_err) => {
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
