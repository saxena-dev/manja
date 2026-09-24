//! GTT API group: `/gtt/triggers` (`kite:gtt.md`).
//!
//! A GTT (Good Till Triggered) order is a trigger held by the broker: when
//! the price reaches a trigger value, the broker places the matching LIMIT
//! order. Placement, modification and deletion make exactly one transport
//! attempt per call, including after an HTTP 429 or a lost response, and are
//! validated before admission, so an invalid request sends nothing. The
//! returned [`GttReceipt`] names the trigger; it does not show that the
//! trigger fired or that an order was placed. A lost or malformed response
//! after dispatch does not show whether the broker acted: the error's stage
//! says so, and [`Gtt::get_trigger`] reports the trigger's state.
//!
//! Each mutation has a `*_with_permit` variant that dispatches with a
//! [`DispatchPermit`] obtained from `HTTPClient::admit`, as the order
//! mutations do. A GTT permit names the operation, not the trigger: GTT
//! mutations share the standard quota class, with no per-trigger limit.
//!
//! The documented sandbox does not offer GTT (`kite:sandbox.md:298`).
//!
use crate::kite::connect::{
    client::HTTPClient,
    models::{GttReceipt, GttRequest, GttTrigger, KiteApiResponse},
    scheduler::DispatchPermit,
};
use crate::kite::error::Result;

/// GTT placement, modification, deletion and retrieval.
pub struct Gtt<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
}

impl<'c> Gtt<'c> {
    /// GTT APIs on `client`.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self { client }
    }

    // ===== [ Mutations: one attempt each ] =====

    /// Place a GTT: `POST /gtt/triggers`, form-encoded.
    ///
    /// The request is validated first; an invalid one is a `Validation`
    /// error and nothing is sent.
    pub async fn place_trigger(&self, request: &GttRequest) -> Result<KiteApiResponse<GttReceipt>> {
        self.place(request, None).await
    }

    /// [`Self::place_trigger`] with admitted capacity from
    /// `HTTPClient::admit(PermitTarget::PlaceGtt)`.
    pub async fn place_trigger_with_permit(
        &self,
        request: &GttRequest,
        permit: DispatchPermit,
    ) -> Result<KiteApiResponse<GttReceipt>> {
        self.place(request, Some(permit)).await
    }

    async fn place(
        &self,
        request: &GttRequest,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<GttReceipt>> {
        self.client
            .send_form(
                reqwest::Method::POST,
                "/gtt/triggers",
                request.validate(),
                request.form_pairs(),
                permit,
            )
            .await
    }

    /// Replace an active GTT: `PUT /gtt/triggers/{trigger_id}`, form-encoded
    /// with the complete new trigger (`kite:gtt.md:373-393`).
    pub async fn modify_trigger(
        &self,
        trigger_id: u64,
        request: &GttRequest,
    ) -> Result<KiteApiResponse<GttReceipt>> {
        self.modify(trigger_id, request, None).await
    }

    /// [`Self::modify_trigger`] with admitted capacity from
    /// `HTTPClient::admit(PermitTarget::ModifyGtt)`.
    pub async fn modify_trigger_with_permit(
        &self,
        trigger_id: u64,
        request: &GttRequest,
        permit: DispatchPermit,
    ) -> Result<KiteApiResponse<GttReceipt>> {
        self.modify(trigger_id, request, Some(permit)).await
    }

    async fn modify(
        &self,
        trigger_id: u64,
        request: &GttRequest,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<GttReceipt>> {
        self.client
            .send_form(
                reqwest::Method::PUT,
                &format!("/gtt/triggers/{trigger_id}"),
                request.validate(),
                request.form_pairs(),
                permit,
            )
            .await
    }

    /// Delete an active GTT: `DELETE /gtt/triggers/{trigger_id}`.
    pub async fn delete_trigger(&self, trigger_id: u64) -> Result<KiteApiResponse<GttReceipt>> {
        self.delete(trigger_id, None).await
    }

    /// [`Self::delete_trigger`] with admitted capacity from
    /// `HTTPClient::admit(PermitTarget::DeleteGtt)`.
    pub async fn delete_trigger_with_permit(
        &self,
        trigger_id: u64,
        permit: DispatchPermit,
    ) -> Result<KiteApiResponse<GttReceipt>> {
        self.delete(trigger_id, Some(permit)).await
    }

    async fn delete(
        &self,
        trigger_id: u64,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<GttReceipt>> {
        self.client
            .send_form(
                reqwest::Method::DELETE,
                &format!("/gtt/triggers/{trigger_id}"),
                Ok(()),
                Vec::new(),
                permit,
            )
            .await
    }

    // ===== [ Reads ] =====

    /// Active GTTs, and GTTs in other states from the previous 7 days:
    /// `GET /gtt/triggers` (`kite:gtt.md:169-172`).
    pub async fn list_triggers(&self) -> Result<KiteApiResponse<Vec<GttTrigger>>> {
        self.client.get("/gtt/triggers").await
    }

    /// One GTT, whatever its age or state: `GET /gtt/triggers/{trigger_id}`
    /// (`kite:gtt.md:281-284`).
    pub async fn get_trigger(&self, trigger_id: u64) -> Result<KiteApiResponse<GttTrigger>> {
        self.client
            .get(&format!("/gtt/triggers/{trigger_id}"))
            .await
    }
}
