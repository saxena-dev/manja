//! Margin and charges calculation API group: `/margins/` and `/charges/`.
//!
//! Order margins, basket margins and the virtual contract note are JSON
//! POST calculations (`kite-api-docs/docs/connect/v3/margins.md:1-13`). They
//! change no order or position, so the scheduler classifies them as `Calc`
//! and may retry them after 429, 502, 503, 504 or a transport fault, within
//! the operation deadline, like reads.
//!
//! Every request is validated before admission: an empty list, an invalid
//! order or a body over `B-HTTP-09` sends nothing. Responses are returned as
//! the broker sent them; no entry is invented for an order the response
//! omits.
//!
//! The documented sandbox excludes every margin calculation endpoint (arch
//! §21.3, D1), so these contracts are verified only against local fixtures.
//! No test or example falls back to a sandbox or production host.
//!
use crate::kite::connect::{
    client::HTTPClient,
    models::{
        BasketMargin, KiteApiResponse, OrderCharges, OrderChargesRequest, OrderMargin,
        OrderMarginRequest, RequestError,
    },
};
use crate::kite::error::Result;

fn non_empty<T>(items: &[T]) -> std::result::Result<(), RequestError> {
    if items.is_empty() {
        Err(RequestError {
            field: "orders",
            reason: "at least one order is required",
        })
    } else {
        Ok(())
    }
}

/// Order and basket margin calculations.
pub struct Margins<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
}

impl<'c> Margins<'c> {
    /// Margin APIs on `client`.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self { client }
    }

    /// Margins for each order, considering existing positions and open
    /// orders: `POST /margins/orders` with a JSON array. The response is an
    /// array with the broker's entries.
    pub async fn orders(
        &self,
        orders: &[OrderMarginRequest],
    ) -> Result<KiteApiResponse<Vec<OrderMargin>>> {
        let valid = non_empty(orders).and_then(|_| orders.iter().try_for_each(|o| o.validate()));
        self.client
            .send_json(reqwest::Method::POST, "/margins/orders", valid, orders)
            .await
    }

    /// Margins for a basket of orders, including the spread benefit:
    /// `POST /margins/basket?consider_positions={bool}` with a JSON array.
    pub async fn basket(
        &self,
        orders: &[OrderMarginRequest],
        consider_positions: bool,
    ) -> Result<KiteApiResponse<BasketMargin>> {
        let valid = non_empty(orders).and_then(|_| orders.iter().try_for_each(|o| o.validate()));
        self.client
            .send_json(
                reqwest::Method::POST,
                &format!("/margins/basket?consider_positions={consider_positions}"),
                valid,
                orders,
            )
            .await
    }
}

/// Order-wise charges: the virtual contract note.
pub struct Charges<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
}

impl<'c> Charges<'c> {
    /// Charges APIs on `client`.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self { client }
    }

    /// Brokerage, STT, stamp duty, exchange, SEBI and GST charges per order:
    /// `POST /charges/orders` with a JSON array.
    pub async fn orders(
        &self,
        orders: &[OrderChargesRequest],
    ) -> Result<KiteApiResponse<Vec<OrderCharges>>> {
        let valid = non_empty(orders).and_then(|_| orders.iter().try_for_each(|o| o.validate()));
        self.client
            .send_json(reqwest::Method::POST, "/charges/orders", valid, orders)
            .await
    }
}
