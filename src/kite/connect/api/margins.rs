//! Margin and charges calculation API group: `/margins/` and `/charges/`.
//!
//! Order margins, basket margins and the virtual contract note are JSON
//! POST calculations (`kite:margins.md:1-13`). They
//! change no order or position, so the scheduler classifies them as `Calc`
//! and may retry them after 429, 502, 503, 504 or a transport fault, within
//! the operation deadline, like reads.
//!
//! Every request is validated before admission: an empty list, an invalid
//! order or a body over `B-HTTP-09` sends nothing. Responses are returned as
//! the broker sent them; no entry is invented for an order the response
//! omits.
//!
//! The documented sandbox excludes every margin calculation endpoint
//! (`kite:sandbox.md:294-300`), so these contracts are verified only
//! against local fixtures.
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

/// Margin calculations for orders you might place, one by one or as a
/// basket. Borrowed from a client with
/// [`HTTPClient::margins`](crate::kite::connect::client::HTTPClient::margins).
/// Calculations change nothing, so they retry like reads.
///
/// # Example
///
/// ```no_run
/// use manja::kite::connect::models::{
///     Exchange, OrderMarginRequest, OrderType, OrderVariety, ProductType, TransactionType,
/// };
/// use manja::kite::protocol::Quantity;
/// use manja::kite::connect::client::HTTPClient;
/// use manja::kite::connect::config::Config;
/// use manja::kite::connect::credentials::Credentials;
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let client = HTTPClient::new(Config::default())?
///     .with_credentials(Credentials::new("api_key", "access_token")?);
/// let order = OrderMarginRequest {
///     exchange: Exchange::NSE,
///     tradingsymbol: "INFY".into(),
///     transaction_type: TransactionType::BUY,
///     variety: OrderVariety::Regular,
///     product: ProductType::CashAndCarry,
///     order_type: OrderType::Market,
///     quantity: Quantity::new(1)?,
///     price: 0.0,
///     trigger_price: 0.0,
/// };
/// for margin in client.margins().orders(&[order]).await?.data.unwrap_or_default() {
///     println!("{}: total {}", margin.tradingsymbol, margin.total);
/// }
/// # Ok(()) }
/// ```
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

/// Order-wise charges, the virtual contract note: brokerage, taxes and fees
/// for orders that executed. Borrowed from a client with
/// [`HTTPClient::charges`](crate::kite::connect::client::HTTPClient::charges).
/// Calculations change nothing, so they retry like reads.
///
/// # Example
///
/// ```no_run
/// use manja::kite::connect::models::{
///     Exchange, OrderChargesRequest, OrderType, OrderVariety, ProductType, TransactionType,
/// };
/// use manja::kite::protocol::Quantity;
/// use manja::kite::connect::client::HTTPClient;
/// use manja::kite::connect::config::Config;
/// use manja::kite::connect::credentials::Credentials;
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let client = HTTPClient::new(Config::default())?
///     .with_credentials(Credentials::new("api_key", "access_token")?);
/// let order = OrderChargesRequest {
///     // Any label for this order in the calculation.
///     order_id: "rebalance42".into(),
///     exchange: Exchange::NSE,
///     tradingsymbol: "INFY".into(),
///     transaction_type: TransactionType::BUY,
///     variety: OrderVariety::Regular,
///     product: ProductType::CashAndCarry,
///     order_type: OrderType::Market,
///     quantity: Quantity::new(1)?,
///     average_price: 1500.0,
/// };
/// for charged in client.charges().orders(&[order]).await?.data.unwrap_or_default() {
///     println!("{}: {} in charges", charged.tradingsymbol, charged.charges.total);
/// }
/// # Ok(()) }
/// ```
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
