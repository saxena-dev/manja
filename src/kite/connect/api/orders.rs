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
//! Each mutation has a `*_with_permit` variant that dispatches with a
//! [`DispatchPermit`] obtained from `HTTPClient::admit`, giving the caller an
//! explicit point between admission and dispatch.
//!
use crate::kite::connect::{
    client::{HTTPClient, RowPolicy},
    models::{
        KiteApiResponse, ModifyOrderRequest, Order, OrderReceipt, OrderVariety, PlaceOrderRequest,
        Row, Rows, Trade,
    },
    scheduler::DispatchPermit,
};
use crate::kite::error::Result;
use crate::kite::protocol::OrderId;

/// Order placement, modification and cancellation, the order book and the
/// trade book.
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
    /// error and nothing is sent.
    pub async fn place_order(
        &self,
        request: &PlaceOrderRequest,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.place(request, None).await
    }

    /// [`Self::place_order`] with admitted capacity from
    /// `HTTPClient::admit(PermitTarget::PlaceOrder)`.
    pub async fn place_order_with_permit(
        &self,
        request: &PlaceOrderRequest,
        permit: DispatchPermit,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.place(request, Some(permit)).await
    }

    async fn place(
        &self,
        request: &PlaceOrderRequest,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.client
            .send_form(
                reqwest::Method::POST,
                &format!("/orders/{}", request.variety),
                request.validate(),
                request.form_pairs(),
                permit,
            )
            .await
    }

    /// Modify an open or pending order: `PUT /orders/{variety}/{order_id}`,
    /// form-encoded, sending only the fields that are set.
    pub async fn modify_order(
        &self,
        variety: OrderVariety,
        order_id: &OrderId,
        request: &ModifyOrderRequest,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.modify(variety, order_id, request, None).await
    }

    /// [`Self::modify_order`] with admitted capacity from
    /// `HTTPClient::admit(PermitTarget::ModifyOrder { order_id })`.
    pub async fn modify_order_with_permit(
        &self,
        variety: OrderVariety,
        order_id: &OrderId,
        request: &ModifyOrderRequest,
        permit: DispatchPermit,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.modify(variety, order_id, request, Some(permit)).await
    }

    async fn modify(
        &self,
        variety: OrderVariety,
        order_id: &OrderId,
        request: &ModifyOrderRequest,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.client
            .send_form(
                reqwest::Method::PUT,
                &format!("/orders/{variety}/{order_id}"),
                request.validate(variety),
                request.form_pairs(),
                permit,
            )
            .await
    }

    /// Cancel an open or pending order: `DELETE /orders/{variety}/{order_id}`.
    ///
    /// Only the `Authorization` header authenticates the request; neither
    /// the API key nor the access token is placed in the URL
    /// (`kite:orders.md:147-162`). The receipt acknowledges the request, not a
    /// confirmed cancellation.
    pub async fn cancel_order(
        &self,
        variety: OrderVariety,
        order_id: &OrderId,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.cancel(variety, order_id, None).await
    }

    /// [`Self::cancel_order`] with admitted capacity from
    /// `HTTPClient::admit(PermitTarget::CancelOrder)`.
    pub async fn cancel_order_with_permit(
        &self,
        variety: OrderVariety,
        order_id: &OrderId,
        permit: DispatchPermit,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.cancel(variety, order_id, Some(permit)).await
    }

    async fn cancel(
        &self,
        variety: OrderVariety,
        order_id: &OrderId,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.client
            .send_form(
                reqwest::Method::DELETE,
                &format!("/orders/{variety}/{order_id}"),
                Ok(()),
                Vec::new(),
                permit,
            )
            .await
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
    pub async fn list_orders(&self) -> Result<KiteApiResponse<Vec<Order>>> {
        strict(self.client.get_rows("/orders", RowPolicy::Strict).await?)
    }

    /// Every order of the day, row by row: `GET /orders`.
    ///
    /// The response may be partial: rows that do not decode are
    /// [`Row::Rejected`](crate::kite::connect::models::Row::Rejected) in
    /// place, in broker order. Every row can be rejected, which leaves
    /// [`Rows::items`] empty although the broker sent orders; check
    /// [`Rows::is_complete`] or use [`Rows::into_complete`] to tell that
    /// apart from an empty order book.
    pub async fn list_orders_with_rejections(&self) -> Result<KiteApiResponse<Rows<Order>>> {
        self.client.get_rows("/orders", RowPolicy::Tolerant).await
    }

    /// The history of one order: `GET /orders/{order_id}`.
    ///
    /// Strict, like [`Self::list_orders`].
    pub async fn get_order_history(
        &self,
        order_id: &OrderId,
    ) -> Result<KiteApiResponse<Vec<Order>>> {
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
    ) -> Result<KiteApiResponse<Rows<Order>>> {
        self.client
            .get_rows(&format!("/orders/{order_id}"), RowPolicy::Tolerant)
            .await
    }

    /// Every executed trade of the day: `GET /trades`.
    ///
    /// Strict, like [`Self::list_orders`].
    pub async fn list_trades(&self) -> Result<KiteApiResponse<Vec<Trade>>> {
        strict(self.client.get_rows("/trades", RowPolicy::Strict).await?)
    }

    /// Every executed trade of the day, row by row: `GET /trades`.
    ///
    /// May be partial, like [`Self::list_orders_with_rejections`].
    pub async fn list_trades_with_rejections(&self) -> Result<KiteApiResponse<Rows<Trade>>> {
        self.client.get_rows("/trades", RowPolicy::Tolerant).await
    }

    /// The trades of one order: `GET /orders/{order_id}/trades`.
    ///
    /// Strict, like [`Self::list_orders`].
    pub async fn get_order_trades(
        &self,
        order_id: &OrderId,
    ) -> Result<KiteApiResponse<Vec<Trade>>> {
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
    ) -> Result<KiteApiResponse<Rows<Trade>>> {
        self.client
            .get_rows(&format!("/orders/{order_id}/trades"), RowPolicy::Tolerant)
            .await
    }
}

/// The rows of a strict response as a plain list. Strict decoding has already
/// failed the operation if any row was rejected, so every row here decoded;
/// the debug assertion guards that invariant against a future change.
fn strict<T>(response: KiteApiResponse<Rows<T>>) -> Result<KiteApiResponse<Vec<T>>> {
    Ok(KiteApiResponse {
        status: response.status,
        data: response.data.map(|rows| {
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
        }),
        message: response.message,
        error_type: response.error_type,
    })
}
