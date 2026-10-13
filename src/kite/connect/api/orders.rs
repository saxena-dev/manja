//! Orders API group: `/orders/` and `/trades`.
//!
//! Placing an order registers it with the OMS. That does not guarantee its
//! receipt at the exchange, and its status is not known when placement
//! returns (`kite:orders.md:42-52`). The
//! acknowledgements returned here ([`OrderReceipt`]) therefore assert no
//! fill, no final modification and no confirmed cancellation; the order book,
//! order history and order updates report what happened.
//!
//! Placement, modification and cancellation make exactly one transport
//! attempt per call, including after an HTTP 429 or a lost response, and are
//! validated before admission, so an invalid request sends nothing. A lost
//! or malformed response after dispatch does not show whether the broker
//! acted: the error's stage says so, and the caller decides what to do next.
//!
//! Order IDs are [`OrderId`]s, valid by construction, so no caller string
//! can change a request path.
//!
use crate::kite::connect::{
    client::{HTTPClient, RowPolicy},
    dispatch::{Dispatched, ReceiptIds},
    models::{
        ModifyOrderRequest, Order, OrderReceipt, OrderVariety, PlaceOrderRequest, Response, Row,
        Rows, Tag, Trade,
    },
    wire::WireForm,
};
use crate::kite::error::Result;
use crate::kite::protocol::OrderId;

/// Order placement, modification and cancellation, the order book and the
/// trade book. Borrowed from a client with
/// [`HTTPClient::orders`](crate::kite::connect::client::HTTPClient::orders).
///
/// Placement, modification and cancellation make exactly one attempt. If one
/// fails after the request may have reached Kite
/// ([`HttpError::may_have_reached_broker`](crate::kite::error::HttpError::may_have_reached_broker)),
/// look the order up before sending it again: [`Self::list_orders_tagged`]
/// finds the rows carrying the placement's [`Tag`], which every placement must
/// carry. A receipt means Kite accepted the request, not that the order
/// filled.
///
/// # Example
///
/// ```no_run
/// use manja::kite::connect::client::HTTPClient;
/// use manja::kite::connect::config::Config;
/// use manja::kite::connect::credentials::Credentials;
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let client = HTTPClient::new(Config::default())?
///     .with_credentials(Credentials::new("api_key", "access_token")?);
/// for order in client.orders().list_orders().await?.data {
///     let history = client.orders().get_order_history(&order.order_id).await?;
///     println!("{}: {} states", order.order_id, history.data.len());
/// }
/// # Ok(()) }
/// ```
pub struct Orders<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
}

impl<'c> Orders<'c> {
    /// Order APIs on `client`.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self { client }
    }

    // ===== [ Mutations: one attempt each ] =====

    /// Place an order: `POST /orders/{variety}`, form-encoded.
    ///
    /// The request is validated first; an invalid one is a `Validation`
    /// error and nothing is sent. The bytes sent are exactly those of
    /// [`PlaceOrderRequest::wire`].
    pub fn place_order(&self, request: &PlaceOrderRequest) -> Dispatched<OrderReceipt> {
        self.client
            .dispatch(request.wire(), true, ReceiptIds::of_order)
    }

    /// Modify an open or pending order: `PUT /orders/{variety}/{order_id}`,
    /// form-encoded, sending only the fields that are set. The bytes sent
    /// are exactly those of [`ModifyOrderRequest::wire`].
    pub fn modify_order(
        &self,
        variety: OrderVariety,
        order_id: &OrderId,
        request: &ModifyOrderRequest,
    ) -> Dispatched<OrderReceipt> {
        self.client
            .dispatch(request.wire(variety, order_id), true, ReceiptIds::of_order)
    }

    /// Cancel an open or pending order: `DELETE /orders/{variety}/{order_id}`.
    ///
    /// Only the `Authorization` header authenticates the request; neither
    /// the API key nor the access token is placed in the URL
    /// (`kite:orders.md:147-162`). The receipt acknowledges the request, not a
    /// confirmed cancellation. The request is [`WireForm::cancel_order`].
    pub fn cancel_order(
        &self,
        variety: OrderVariety,
        order_id: &OrderId,
    ) -> Dispatched<OrderReceipt> {
        self.client.dispatch(
            Ok(WireForm::cancel_order(variety, order_id)),
            true,
            ReceiptIds::of_order,
        )
    }

    // ===== [ Reads ] =====
    //
    // Each list has a strict method, which fails the whole response when any
    // row does not decode, and a `*_with_rejections` method, which returns
    // every row in broker order with rejected rows in place.

    /// Every order of the day, open and executed: `GET /orders`.
    ///
    /// Strict: if any order does not decode, the whole response is a
    /// `Decode` error whose detail gives the rejected count and the first
    /// index. [`Self::list_orders_with_rejections`] returns the other rows.
    pub async fn list_orders(&self) -> Result<Response<Vec<Order>>> {
        strict(self.client.get_rows("/orders", RowPolicy::Strict).await?)
    }

    /// The orders of the day carrying `tag`: `GET /orders`, filtered here.
    ///
    /// The broker echoes a placement's [`Tag`] on the order it registered,
    /// so this is how a placement whose response was lost is looked for.
    /// It is a filter, not proof: Kite does not enforce tag uniqueness and
    /// an order placed elsewhere can carry any text, so every row with the
    /// tag is returned, in broker order, and the caller weighs them. The
    /// whole order book is fetched and decoded strictly, as by
    /// [`Self::list_orders`]; the broker has no tag query.
    pub async fn list_orders_tagged(&self, tag: &Tag) -> Result<Response<Vec<Order>>> {
        let book = self.list_orders().await?;
        Ok(book.map(|orders| {
            orders
                .into_iter()
                .filter(|order| order.tag.as_deref() == Some(tag.as_str()))
                .collect()
        }))
    }

    /// Every order of the day, row by row: `GET /orders`.
    ///
    /// The response may be partial: rows that do not decode are
    /// [`Row::Rejected`](crate::kite::connect::models::Row::Rejected) in
    /// place, in broker order. Every row can be rejected, which leaves
    /// [`Rows::items`] empty although the broker sent orders; check
    /// [`Rows::is_complete`] or use [`Rows::into_complete`] to tell that
    /// apart from an empty order book.
    pub async fn list_orders_with_rejections(&self) -> Result<Response<Rows<Order>>> {
        self.client.get_rows("/orders", RowPolicy::Tolerant).await
    }

    /// The history of one order: `GET /orders/{order_id}`.
    ///
    /// Strict, like [`Self::list_orders`].
    pub async fn get_order_history(&self, order_id: &OrderId) -> Result<Response<Vec<Order>>> {
        strict(
            self.client
                .get_rows(&format!("/orders/{order_id}"), RowPolicy::Strict)
                .await?,
        )
    }

    /// The history of one order, row by row: `GET /orders/{order_id}`.
    ///
    /// May be partial, like [`Self::list_orders_with_rejections`]: knowing
    /// the order ID does not stop any other field of a row from failing.
    pub async fn get_order_history_with_rejections(
        &self,
        order_id: &OrderId,
    ) -> Result<Response<Rows<Order>>> {
        self.client
            .get_rows(&format!("/orders/{order_id}"), RowPolicy::Tolerant)
            .await
    }

    /// Every executed trade of the day: `GET /trades`.
    ///
    /// Strict, like [`Self::list_orders`].
    pub async fn list_trades(&self) -> Result<Response<Vec<Trade>>> {
        strict(self.client.get_rows("/trades", RowPolicy::Strict).await?)
    }

    /// Every executed trade of the day, row by row: `GET /trades`.
    ///
    /// May be partial, like [`Self::list_orders_with_rejections`].
    pub async fn list_trades_with_rejections(&self) -> Result<Response<Rows<Trade>>> {
        self.client.get_rows("/trades", RowPolicy::Tolerant).await
    }

    /// The trades of one order: `GET /orders/{order_id}/trades`.
    ///
    /// Strict, like [`Self::list_orders`].
    pub async fn get_order_trades(&self, order_id: &OrderId) -> Result<Response<Vec<Trade>>> {
        strict(
            self.client
                .get_rows(&format!("/orders/{order_id}/trades"), RowPolicy::Strict)
                .await?,
        )
    }

    /// The trades of one order, row by row: `GET /orders/{order_id}/trades`.
    ///
    /// May be partial, like [`Self::list_orders_with_rejections`].
    pub async fn get_order_trades_with_rejections(
        &self,
        order_id: &OrderId,
    ) -> Result<Response<Rows<Trade>>> {
        self.client
            .get_rows(&format!("/orders/{order_id}/trades"), RowPolicy::Tolerant)
            .await
    }
}

/// The rows of a strict response as a plain list. Strict decoding has already
/// failed the operation if any row was rejected, so every row here decoded;
/// the debug assertion guards that invariant against a future change.
fn strict<T>(response: Response<Rows<T>>) -> Result<Response<Vec<T>>> {
    Ok(response.map(|rows| {
        debug_assert!(
            rows.is_complete(),
            "strict decoding returned a rejected row"
        );
        rows.rows
            .into_iter()
            .filter_map(|row| match row {
                Row::Decoded(t) => Some(t),
                Row::Rejected(_) => None,
            })
            .collect()
    }))
}
