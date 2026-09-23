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
//! Each mutation has a `*_with_permit` variant that dispatches with a
//! [`DispatchPermit`] obtained from `HTTPClient::admit`, giving the caller an
//! explicit point between admission and dispatch.
//!
use crate::kite::connect::{
    client::HTTPClient,
    models::{
        check_order_id, KiteApiResponse, ModifyOrderRequest, Order, OrderReceipt, OrderVariety,
        PlaceOrderRequest, Trade,
    },
    scheduler::DispatchPermit,
};
use crate::kite::error::Result;

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
        order_id: &str,
        request: &ModifyOrderRequest,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.modify(variety, order_id, request, None).await
    }

    /// [`Self::modify_order`] with admitted capacity from
    /// `HTTPClient::admit(PermitTarget::ModifyOrder { order_id })`.
    pub async fn modify_order_with_permit(
        &self,
        variety: OrderVariety,
        order_id: &str,
        request: &ModifyOrderRequest,
        permit: DispatchPermit,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.modify(variety, order_id, request, Some(permit)).await
    }

    async fn modify(
        &self,
        variety: OrderVariety,
        order_id: &str,
        request: &ModifyOrderRequest,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        let valid = check_order_id(order_id).and_then(|_| request.validate(variety));
        let path = if valid.is_ok() {
            format!("/orders/{variety}/{order_id}")
        } else {
            format!("/orders/{variety}/invalid")
        };
        self.client
            .send_form(
                reqwest::Method::PUT,
                &path,
                valid,
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
        order_id: &str,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.cancel(variety, order_id, None).await
    }

    /// [`Self::cancel_order`] with admitted capacity from
    /// `HTTPClient::admit(PermitTarget::CancelOrder)`.
    pub async fn cancel_order_with_permit(
        &self,
        variety: OrderVariety,
        order_id: &str,
        permit: DispatchPermit,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        self.cancel(variety, order_id, Some(permit)).await
    }

    async fn cancel(
        &self,
        variety: OrderVariety,
        order_id: &str,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<OrderReceipt>> {
        let valid = check_order_id(order_id);
        let path = if valid.is_ok() {
            format!("/orders/{variety}/{order_id}")
        } else {
            format!("/orders/{variety}/invalid")
        };
        self.client
            .send_form(reqwest::Method::DELETE, &path, valid, Vec::new(), permit)
            .await
    }

    // ===== [ Reads ] =====

    /// Every order of the day, open and executed: `GET /orders`.
    pub async fn list_orders(&self) -> Result<KiteApiResponse<Vec<Order>>> {
        self.client.get("/orders").await
    }

    /// The history of one order: `GET /orders/{order_id}`.
    pub async fn get_order_history(&self, order_id: &str) -> Result<KiteApiResponse<Vec<Order>>> {
        self.client.get(&format!("/orders/{order_id}")).await
    }

    /// Every executed trade of the day: `GET /trades`.
    pub async fn list_trades(&self) -> Result<KiteApiResponse<Vec<Trade>>> {
        self.client.get("/trades").await
    }

    /// The trades of one order: `GET /orders/{order_id}/trades`.
    pub async fn get_order_trades(&self, order_id: &str) -> Result<KiteApiResponse<Vec<Trade>>> {
        self.client.get(&format!("/orders/{order_id}/trades")).await
    }
}
