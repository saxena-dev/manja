//! Margin calculation API group: `/margins/` and `/charges/`
//!
//! This module provides functionality to calculate margin requirements and charges
//! such as `span`, `exposure`, `option premium`, `additional`, `bo`, `cash`, `var`
//! and `pnl` values for a list of orders from the respective endpoints of Kite Connect API.
//!
//! Refer to the official API [documentation](https://kite.trade/docs/connect/v3/market-quotes/).
//!
use crate::kite::connect::api::{create_backoff_policy, BackoffPolicy};
use crate::kite::connect::{
    client::HTTPClient,
    models::{
        BasketMargin, KiteApiResponse, OrderCharges, OrderChargesRequest, OrderMargin,
        OrderMarginRequest,
    },
};
use crate::kite::error::Result;

/// Margin calculation APIs lets you calculate `span`, `exposure`, `option premium`,
/// `additional`, `bo`, `cash`, `var`, `pnl` values for a list of orders.
///
pub struct Margins<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Margins<'c> {
    /// Creates a new instance of `Margins` with default API rate limits.
    ///
    /// # Arguments
    ///
    /// * `client` - A reference to the `HTTPClient` used for making API requests.
    ///
    /// # Returns
    ///
    /// A new instance of `Margins`.
    ///
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Margins` instance.
    ///
    /// # Arguments
    ///
    /// * `backoff` - A `BackoffPolicy` instance specifying the backoff policy.
    ///
    /// # Returns
    ///
    /// The `Margins` instance with the updated backoff policy.
    ///
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Calculates margins for each order considering the existing positions
    /// and open orders.
    ///
    pub async fn orders(
        &self,
        request: OrderMarginRequest,
    ) -> Result<KiteApiResponse<OrderMargin>> {
        self.client
            .post(&"/margins/orders", request, &self.backoff)
            .await
    }

    /// Calculates margins for spread orders.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let order_reqs = vec![
    ///     OrderMarginRequest {
    ///         exchange: Exchange::NSE,
    ///         tradingsymbol: format!("SBIN"),
    ///         transaction_type: TransactionType::SELL,
    ///         variety: OrderVariety::Regular,
    ///         product: ProductType::MarginIntradaySquareoff,
    ///         order_type: OrderType::Market,
    ///         quantity: 100,
    ///         price: 0.0,
    ///         trigger_price: 0.0,
    ///     },
    ///     OrderMarginRequest {
    ///         exchange: Exchange::NSE,
    ///         tradingsymbol: format!("AXISBANK"),
    ///         transaction_type: TransactionType::BUY,
    ///         variety: OrderVariety::Regular,
    ///         product: ProductType::MarginIntradaySquareoff,
    ///         order_type: OrderType::Market,
    ///         quantity: 100,
    ///         price: 0.0,
    ///         trigger_price: 0.0,
    ///     },
    /// ];
    ///
    /// let resp = manja_client.margins().basket(&order_reqs, true).await?;
    /// info!("Basket margins:\n\n{:?}", resp);
    /// ```
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
///
pub struct Charges<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Charges<'c> {
    /// Creates a new instance of `Charges` with default API rate limits.
    ///
    /// # Arguments
    ///
    /// * `client` - A reference to the `HTTPClient` used for making API requests.
    ///
    /// # Returns
    ///
    /// A new instance of `Charges`.
    ///
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Charges` instance.
    ///
    /// # Arguments
    ///
    /// * `backoff` - A `BackoffPolicy` instance specifying the backoff policy.
    ///
    /// # Returns
    ///
    /// The `Charges` instance with the updated backoff policy.
    ///
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Calculates order-wise charges for orderbook.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let order_charges_requests = vec![
    ///     OrderChargesRequest {
    ///         order_id: String::from("111111111"),
    ///         exchange: Exchange::NSE,
    ///         tradingsymbol: String::from("SBIN"),
    ///         transaction_type: TransactionType::BUY,
    ///         variety: OrderVariety::Regular,
    ///         product: ProductType::CashAndCarry,
    ///         order_type: OrderType::Market,
    ///         quantity: 1,
    ///         average_price: 560.0,
    ///     },
    ///     OrderChargesRequest {
    ///         order_id: String::from("2222222222"),
    ///         exchange: Exchange::MCX,
    ///         tradingsymbol: String::from("GOLDPETAL24AUGFUT"),
    ///         transaction_type: TransactionType::SELL,
    ///         variety: OrderVariety::Regular,
    ///         product: ProductType::Normal,
    ///         order_type: OrderType::Limit,
    ///         quantity: 1,
    ///         average_price: 5862.0,
    ///     },
    ///     OrderChargesRequest {
    ///         order_id: String::from("3333333333"),
    ///         exchange: Exchange::NFO,
    ///         tradingsymbol: String::from("ADANIPORTS24JUL1460CE"),
    ///         transaction_type: TransactionType::BUY,
    ///         variety: OrderVariety::Regular,
    ///         product: ProductType::Normal,
    ///         order_type: OrderType::Limit,
    ///         quantity: 100,
    ///         average_price: 1.5,
    ///     },
    /// ];
    ///
    /// let resp = manja_client
    ///     .charges()
    ///     .orders(&order_charges_requests)
    ///     .await?;
    /// info!("Virtual contract note:\n\n{:?}", resp);
    /// ```
    ///
    pub async fn orders(
        &self,
        requests: &[OrderChargesRequest],
    ) -> Result<KiteApiResponse<Vec<OrderCharges>>> {
        self.client
            .post(&"/charges/orders", requests, &self.backoff)
            .await
    }
}
