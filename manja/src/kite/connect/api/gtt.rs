//! GTT API group facade: `/gtt/triggers`
//!
//! This module provides the facade-layer API group for interacting with the
//! GTT (Good Till Triggered) endpoints of the Kite Connect HTTP API.

use std::collections::HashMap;

use serde_json::json;

use crate::kite::connect::api::{create_backoff_policy, BackoffPolicy};
use crate::kite::connect::client::HTTPClient;
use crate::kite::connect::models::{
    GttTrigger, GttTriggerId, GttTriggerRequest, GttType, KiteApiResponse,
};
use crate::kite::error::Result;

/// The GTT APIs let you place, modify and manage GTT triggers.
pub struct Gtt<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Gtt<'c> {
    /// Creates a new instance of `Gtt` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Gtt` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    fn build_form_payload(&self, req: &GttTriggerRequest) -> HashMap<String, String> {
        let mut form: HashMap<String, String> = HashMap::new();
        let gtt_type_str = match req.gtt_type {
            GttType::Single => "single",
            GttType::TwoLeg => "two-leg",
        };
        form.insert("type".to_string(), gtt_type_str.to_string());
        form.insert(
            "condition".to_string(),
            serde_json::to_string(&req.condition).unwrap_or_else(|_| json!({}).to_string()),
        );
        form.insert(
            "orders".to_string(),
            serde_json::to_string(&req.orders).unwrap_or_else(|_| json!([]).to_string()),
        );
        form
    }

    /// Creates a new GTT trigger.
    pub async fn create_trigger(
        &self,
        req: &GttTriggerRequest,
    ) -> Result<KiteApiResponse<GttTriggerId>> {
        let form = self.build_form_payload(req);
        self.client
            .post_form("/gtt/triggers", &form, &self.backoff)
            .await
    }

    /// Lists all GTT triggers visible in the GTT order book.
    pub async fn list_triggers(&self) -> Result<KiteApiResponse<Vec<GttTrigger>>> {
        self.client
            .get("/gtt/triggers", &self.backoff)
            .await
    }

    /// Retrieves a single GTT trigger by trigger ID.
    pub async fn get_trigger(
        &self,
        trigger_id: i64,
    ) -> Result<KiteApiResponse<GttTrigger>> {
        self.client
            .get(&format!("/gtt/triggers/{}", trigger_id), &self.backoff)
            .await
    }

    /// Modifies an existing active GTT trigger.
    pub async fn modify_trigger(
        &self,
        trigger_id: i64,
        req: &GttTriggerRequest,
    ) -> Result<KiteApiResponse<GttTriggerId>> {
        let form = self.build_form_payload(req);
        self.client
            .put(&format!("/gtt/triggers/{}", trigger_id), form, &self.backoff)
            .await
    }

    /// Deletes an active GTT trigger.
    pub async fn delete_trigger(
        &self,
        trigger_id: i64,
    ) -> Result<KiteApiResponse<GttTriggerId>> {
        self.client
            .delete(&format!("/gtt/triggers/{}", trigger_id), false, &self.backoff)
            .await
    }
}
