//! Alerts API group facade: `/alerts`
//!
//! This module provides the facade-layer API group for interacting with the
//! Alerts endpoints of the Kite Connect HTTP API.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::json;

use crate::kite::connect::api::{create_backoff_policy, BackoffPolicy};
use crate::kite::connect::client::HTTPClient;
use crate::kite::connect::models::{
    Alert, AlertHistoryEntry, AlertListFilter, AlertOperator, AlertRequest, AlertRhsType,
    AlertType, KiteApiResponse,
};
use crate::kite::error::Result;

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
