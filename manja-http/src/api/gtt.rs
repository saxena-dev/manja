//! GTT API group: `/gtt/triggers`
//!
//! This module provides functionality to interact with the GTT (Good Till
//! Triggered) endpoints of the Kite Connect HTTP API.

use std::collections::HashMap;

use serde_json::json;

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::Result;
use manja_core::models::{
    GttTrigger, GttTriggerId, GttTriggerRequest, GttType, KiteApiResponse,
};

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
        form.insert(
            "type".to_string(),
            gtt_type_str.to_string(),
        );
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::join;

    use crate::test_utils::{
        add_mocks, get_http_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
    };
    use manja_core::models::{GttTrigger, GttTriggerId, GttTriggerRequest, GttType};

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(
            ("POST", "/gtt/triggers"),
            "./kiteconnect-mocks/gtt_place_order.json",
        );
        mmap.insert(
            ("GET", "/gtt/triggers"),
            "./kiteconnect-mocks/gtt_get_orders.json",
        );
        mmap.insert(
            ("GET", "/gtt/triggers/123"),
            "./kiteconnect-mocks/gtt_get_order.json",
        );
        mmap.insert(
            ("PUT", "/gtt/triggers/123"),
            "./kiteconnect-mocks/gtt_modify_order.json",
        );
        mmap.insert(
            ("DELETE", "/gtt/triggers/123"),
            "./kiteconnect-mocks/gtt_delete_order.json",
        );
        mmap
    }

    #[tokio::test]
    async fn test_gtt_list_triggers_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client.gtt().list_triggers().await.unwrap();
        let expected =
            read_to_object::<Vec<GttTrigger>>("./kiteconnect-mocks/gtt_get_orders.json").unwrap();

        let triggers = response.data.expect("expected triggers data");
        assert_eq!(triggers.len(), expected.len());
        assert_eq!(triggers[0].id, expected[0].id);
    }

    #[tokio::test]
    async fn test_gtt_get_trigger_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client.gtt().get_trigger(123).await.unwrap();
        let expected =
            read_to_object::<GttTrigger>("./kiteconnect-mocks/gtt_get_order.json").unwrap();

        let trigger = response.data.expect("expected trigger data");
        assert_eq!(trigger.id, expected.id);
        assert_eq!(trigger.user_id, expected.user_id);
    }

    #[tokio::test]
    async fn test_gtt_create_modify_delete_trigger_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let condition_json = r#"{
            "exchange":"NSE",
            "tradingsymbol":"INFY",
            "trigger_values":[702.0],
            "last_price":798.0
        }"#;
        let condition: manja_core::models::GttCondition =
            serde_json::from_str(condition_json).unwrap();
        let order_json = r#"{
            "exchange":"NSE",
            "tradingsymbol":"INFY",
            "transaction_type":"BUY",
            "quantity":1,
            "order_type":"LIMIT",
            "product":"CNC",
            "price":702.5
        }"#;
        let order: manja_core::models::GttOrderParams =
            serde_json::from_str(order_json).unwrap();

        let req = GttTriggerRequest {
            gtt_type: GttType::Single,
            condition,
            orders: vec![order],
        };

        let created: KiteApiResponse<GttTriggerId> =
            client.gtt().create_trigger(&req).await.unwrap();
        let create_id = created.data.expect("expected trigger id").trigger_id;
        assert_eq!(create_id, 123);

        let modified: KiteApiResponse<GttTriggerId> = client
            .gtt()
            .modify_trigger(123, &req)
            .await
            .unwrap();
        let modify_id = modified.data.expect("expected trigger id").trigger_id;
        assert_eq!(modify_id, 123);

        let deleted: KiteApiResponse<GttTriggerId> =
            client.gtt().delete_trigger(123).await.unwrap();
        let delete_id = deleted.data.expect("expected trigger id").trigger_id;
        assert_eq!(delete_id, 123);
    }
}
