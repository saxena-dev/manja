//! Alerts API group: `/alerts`
//!
//! This module provides functionality to interact with the Alerts endpoints of
//! the Kite Connect HTTP API.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::json;

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::Result;
use manja_core::models::{
    Alert, AlertHistoryEntry, AlertListFilter, AlertOperator, AlertRequest, AlertRhsType,
    AlertStatus, AlertType, KiteApiResponse,
};

/// The Alerts APIs let you create, list, query, modify, delete and inspect
/// alert history.
pub struct Alerts<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Alerts<'c> {
    /// Creates a new instance of `Alerts` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Alerts` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    fn build_form_payload(&self, req: &AlertRequest) -> HashMap<String, String> {
        let mut form: HashMap<String, String> = HashMap::new();

        form.insert("name".to_string(), req.name.clone());

        let alert_type_str = match req.alert_type {
            AlertType::Simple => "simple",
            AlertType::Ato => "ato",
        };
        form.insert("type".to_string(), alert_type_str.to_string());

        let lhs_exchange_str: &str = req.lhs_exchange.clone().into();
        form.insert("lhs_exchange".to_string(), lhs_exchange_str.to_string());
        form.insert(
            "lhs_tradingsymbol".to_string(),
            req.lhs_tradingsymbol.clone(),
        );
        form.insert("lhs_attribute".to_string(), req.lhs_attribute.clone());

        let operator_str = match req.operator {
            AlertOperator::LessThan => "<",
            AlertOperator::LessThanOrEqual => "<=",
            AlertOperator::GreaterThan => ">",
            AlertOperator::GreaterThanOrEqual => ">=",
            AlertOperator::Equal => "==",
        };
        form.insert("operator".to_string(), operator_str.to_string());

        let rhs_type_str = match req.rhs_type {
            AlertRhsType::Constant => "constant",
            AlertRhsType::Instrument => "instrument",
        };
        form.insert("rhs_type".to_string(), rhs_type_str.to_string());

        if let Some(rhs_constant) = req.rhs_constant {
            form.insert("rhs_constant".to_string(), rhs_constant.to_string());
        }

        if let Some(rhs_exchange) = &req.rhs_exchange {
            let rhs_exchange_str: &str = rhs_exchange.clone().into();
            if !rhs_exchange_str.is_empty() {
                form.insert("rhs_exchange".to_string(), rhs_exchange_str.to_string());
            }
        }

        if let Some(rhs_tradingsymbol) = &req.rhs_tradingsymbol {
            if !rhs_tradingsymbol.is_empty() {
                form.insert(
                    "rhs_tradingsymbol".to_string(),
                    rhs_tradingsymbol.clone(),
                );
            }
        }

        if let Some(rhs_attribute) = &req.rhs_attribute {
            if !rhs_attribute.is_empty() {
                form.insert("rhs_attribute".to_string(), rhs_attribute.clone());
            }
        }

        if let Some(basket) = &req.basket {
            let basket_json =
                serde_json::to_string(basket).unwrap_or_else(|_| json!({}).to_string());
            form.insert("basket".to_string(), basket_json);
        }

        form
    }

    /// Creates a new alert (simple or ATO).
    pub async fn create_alert(&self, req: &AlertRequest) -> Result<KiteApiResponse<Alert>> {
        let form = self.build_form_payload(req);
        self.client
            .post_form("/alerts", &form, &self.backoff)
            .await
    }

    /// Lists alerts, optionally filtered by status and pagination.
    pub async fn list_alerts(
        &self,
        filter: Option<AlertListFilter>,
    ) -> Result<KiteApiResponse<Vec<Alert>>> {
        match filter {
            Some(f) => self
                .client
                .get_with_query("/alerts", &f, &self.backoff)
                .await,
            None => self.client.get("/alerts", &self.backoff).await,
        }
    }

    /// Retrieves a single alert by UUID.
    pub async fn get_alert(&self, uuid: &str) -> Result<KiteApiResponse<Alert>> {
        self.client
            .get(&format!("/alerts/{}", uuid), &self.backoff)
            .await
    }

    /// Modifies an existing alert by UUID.
    pub async fn modify_alert(
        &self,
        uuid: &str,
        req: &AlertRequest,
    ) -> Result<KiteApiResponse<Alert>> {
        let form = self.build_form_payload(req);
        self.client
            .put(&format!("/alerts/{}", uuid), form, &self.backoff)
            .await
    }

    /// Deletes a single alert by UUID.
    pub async fn delete_alert(&self, uuid: &str) -> Result<KiteApiResponse<Option<()>>> {
        let path = format!("/alerts?uuid={}", uuid);
        self.client
            .delete(&path, false, &self.backoff)
            .await
    }

    /// Deletes multiple alerts by UUIDs.
    pub async fn delete_alerts(&self, uuids: &[String]) -> Result<KiteApiResponse<Option<()>>> {
        let mut query = String::new();
        for (idx, uuid) in uuids.iter().enumerate() {
            if idx > 0 {
                query.push('&');
            }
            query.push_str("uuid=");
            query.push_str(uuid);
        }
        let path = format!("/alerts?{}", query);

        self.client
            .delete(&path, false, &self.backoff)
            .await
    }

    /// Retrieves alert trigger history for a given UUID.
    pub async fn get_alert_history(
        &self,
        uuid: &str,
    ) -> Result<KiteApiResponse<Vec<AlertHistoryEntry>>> {
        self.client
            .get(&format!("/alerts/{}/history", uuid), &self.backoff)
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
    use manja_core::models::{
        Alert, AlertBasket, AlertBasketItem, AlertBasketParams, AlertHistoryEntry, AlertListFilter,
        AlertOperator, AlertRequest, AlertRhsType, AlertStatus, AlertType, Exchange, KiteApiResponse,
        OrderType, ProductType, TransactionType,
    };

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(("POST", "/alerts"), "./kiteconnect-mocks/alerts_create.json");
        mmap.insert(("GET", "/alerts"), "./kiteconnect-mocks/alerts_get.json");
        mmap.insert(
            ("GET", "/alerts/550e8400-e29b-41d4-a716-446655440000"),
            "./kiteconnect-mocks/alerts_get_one.json",
        );
        mmap.insert(
            ("PUT", "/alerts/550e8400-e29b-41d4-a716-446655440000"),
            "./kiteconnect-mocks/alerts_modify.json",
        );
        mmap.insert(
            ("DELETE", "/alerts?uuid=550e8400-e29b-41d4-a716-446655440000"),
            "./kiteconnect-mocks/alerts_delete.json",
        );
        mmap.insert(
            ("DELETE", "/alerts?uuid=550e8400-e29b-41d4-a716-446655440000&uuid=e888ed4a-6801-406f-bdc2-002db5a8411d"),
            "./kiteconnect-mocks/alerts_delete.json",
        );
        mmap.insert(
            ("GET", "/alerts/550e8400-e29b-41d4-a716-446655440000/history"),
            "./kiteconnect-mocks/alerts_history.json",
        );
        mmap
    }

    #[tokio::test]
    async fn test_alerts_list_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client.alerts().list_alerts(None).await.unwrap();
        let expected =
            read_to_object::<Vec<Alert>>("./kiteconnect-mocks/alerts_get.json").unwrap();

        let alerts = response.data.expect("expected alerts data");
        assert_eq!(alerts.len(), expected.len());
        assert_eq!(alerts[0].uuid, expected[0].uuid);
    }

    #[tokio::test]
    async fn test_alerts_get_one_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = client
            .alerts()
            .get_alert("550e8400-e29b-41d4-a716-446655440000")
            .await
            .unwrap();
        let expected =
            read_to_object::<Alert>("./kiteconnect-mocks/alerts_get_one.json").unwrap();

        let alert = response.data.expect("expected alert data");
        assert_eq!(alert.uuid, expected.uuid);
        assert_eq!(alert.alert_type, expected.alert_type);
    }

    #[tokio::test]
    async fn test_alerts_create_and_modify_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let req = AlertRequest {
            name: "NIFTY 50".to_string(),
            alert_type: AlertType::Simple,
            lhs_exchange: Exchange::INDICES,
            lhs_tradingsymbol: "NIFTY 50".to_string(),
            lhs_attribute: "LastTradedPrice".to_string(),
            operator: AlertOperator::GreaterThanOrEqual,
            rhs_type: AlertRhsType::Constant,
            rhs_constant: Some(27000.0),
            rhs_exchange: None,
            rhs_tradingsymbol: None,
            rhs_attribute: None,
            basket: None,
        };

        let created: KiteApiResponse<Alert> =
            client.alerts().create_alert(&req).await.unwrap();
        let alert = created.data.expect("expected created alert");
        assert_eq!(alert.alert_type, AlertType::Simple);

        let modified: KiteApiResponse<Alert> = client
            .alerts()
            .modify_alert("550e8400-e29b-41d4-a716-446655440000", &req)
            .await
            .unwrap();
        let modified_alert = modified.data.expect("expected modified alert");
        assert_eq!(modified_alert.uuid, "550e8400-e29b-41d4-a716-446655440000");
    }

    #[tokio::test]
    async fn test_alerts_delete_single_and_multiple_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let deleted_single: KiteApiResponse<Option<()>> = client
            .alerts()
            .delete_alert("550e8400-e29b-41d4-a716-446655440000")
            .await
            .unwrap();
        assert!(deleted_single.data.is_none());

        let uuids = vec![
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
            "e888ed4a-6801-406f-bdc2-002db5a8411d".to_string(),
        ];
        let deleted_multi: KiteApiResponse<Option<()>> =
            client.alerts().delete_alerts(&uuids).await.unwrap();
        assert!(deleted_multi.data.is_none());
    }

    #[tokio::test]
    async fn test_alerts_history_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response: KiteApiResponse<Vec<AlertHistoryEntry>> = client
            .alerts()
            .get_alert_history("550e8400-e29b-41d4-a716-446655440000")
            .await
            .unwrap();
        let history = response.data.expect("expected history data");
        assert!(!history.is_empty());
        assert_eq!(
            history[0].uuid,
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn test_alerts_list_with_filter_success() {
        let filter = AlertListFilter {
            status: Some(AlertStatus::Enabled),
            page: Some(1),
            page_size: Some(10),
        };

        assert!(matches!(filter.status, Some(AlertStatus::Enabled)));

        let expected =
            read_to_object::<Vec<Alert>>("./kiteconnect-mocks/alerts_get.json").unwrap();
        assert!(!expected.is_empty());
    }
}
