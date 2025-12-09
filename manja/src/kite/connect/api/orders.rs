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
