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

