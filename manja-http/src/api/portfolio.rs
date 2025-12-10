//! Portfolio API group: `/portfolio/`
//!
//! This module provides functionality to interact with holdings and positions
//! endpoints of the Kite Connect HTTP API.

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::Result;
use manja_core::models::{
    Holding, HoldingAuthorisationItem, HoldingsAuthorisationResponse, KiteApiResponse, Positions,
};

/// Portfolio-related API endpoints for holdings and positions.
pub struct Portfolio<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Portfolio<'c> {
    /// Creates a new instance of `Portfolio` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Portfolio` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Fetches the list of holdings for the user.
    pub async fn holdings(&self) -> Result<KiteApiResponse<Vec<Holding>>> {
        self.client.get(&"/portfolio/holdings", &self.backoff).await
    }

    /// Fetches the list of positions for the user.
    pub async fn positions(&self) -> Result<KiteApiResponse<Positions>> {
        self.client
            .get(&"/portfolio/positions", &self.backoff)
            .await
    }

    /// Initiates holdings authorisation at the depository.
    ///
    /// When a sell order requires depository authorisation, this endpoint can
    /// be used to initiate an electronic authorisation flow. The `items`
    /// slice contains optional ISIN/quantity pairs to scope the
    /// authorisation; when empty, the broker may present the entire holdings
    /// portfolio for authorisation.
    pub async fn authorise_holdings(
        &self,
        items: &[HoldingAuthorisationItem],
    ) -> Result<KiteApiResponse<HoldingsAuthorisationResponse>> {
        // Build a form payload with repeated `isin` and `quantity` keys,
        // matching the API's expected encoding:
        //
        // -d "isin=INE002A01018" -d "quantity=50"
        // -d "isin=INE009A01021" -d "quantity=50"
        let mut form: Vec<(String, String)> = Vec::new();
        for item in items {
            form.push(("isin".to_string(), item.isin.clone()));
            form.push((
                "quantity".to_string(),
                item.quantity.to_string(),
            ));
        }

        self.client
            .post_form("/portfolio/holdings/authorise", &form, &self.backoff)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::join;

    use crate::client::HTTPClient;
    use crate::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
    use crate::credentials::KiteCredentials;
    use crate::error::Error;
    use crate::test_utils::{
        add_mocks, get_http_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
    };
    use manja_core::error::KiteApiException;
    use manja_core::models::{
        Holding, HoldingAuthorisationItem, HoldingsAuthorisationResponse,
    };

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(
            ("GET", "/portfolio/holdings"),
            "./kiteconnect-mocks/holdings.json",
        );
        mmap.insert(
            ("POST", "/portfolio/holdings/authorise"),
            "./kiteconnect-mocks/holdings_auth.json",
        );
        mmap
    }

    #[tokio::test]
    async fn test_holdings_success() {
        let (server, client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let portfolio_api = Portfolio::new(&client);
        let response = portfolio_api.holdings().await.unwrap();
        let expected =
            read_to_object::<Vec<Holding>>("./kiteconnect-mocks/holdings.json").unwrap();

        let holdings = response.data.expect("expected holdings data");
        assert_eq!(holdings.len(), expected.len());
        assert_eq!(holdings[0].tradingsymbol, expected[0].tradingsymbol);
    }

    #[tokio::test]
    async fn test_holdings_authorise_success() {
        let (server, client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let portfolio_api = Portfolio::new(&client);
        let items = vec![
            HoldingAuthorisationItem {
                isin: "INE002A01018".to_string(),
                quantity: 50,
            },
            HoldingAuthorisationItem {
                isin: "INE009A01021".to_string(),
                quantity: 50,
            },
        ];

        let response = portfolio_api
            .authorise_holdings(&items)
            .await
            .expect("expected holdings authorisation to succeed");

        let auth: HoldingsAuthorisationResponse = response
            .data
            .expect("expected holdings authorisation data");
        assert_eq!(
            auth.request_id,
            "na8QgCeQm05UHG6NL9sAGRzdfSF64UdB"
        );
    }

    #[tokio::test]
    async fn test_holdings_token_exception_error() {
        let mut server = mockito::Server::new_async().await;
        let credentials = KiteCredentials::new(
            "TEST_API_KEY",
            "TEST_API_SECRET",
            "TEST_USER_ID",
            "TEST_PASSWORD",
            "TEST_TOTP",
        );
        let config = Config::from_parts(
            server.url(),
            KITECONNECT_API_LOGIN.to_string(),
            KITECONNECT_API_REDIRECT.to_string(),
            credentials,
        );
        let client = HTTPClient::with_config(config);

        let _m = server
            .mock("GET", "/portfolio/holdings")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Token is invalid or has expired",
                    "error_type": "TokenException"
                }"#,
            )
            .create_async()
            .await;

        let portfolio_api = Portfolio::new(&client);
        let err = portfolio_api
            .holdings()
            .await
            .expect_err("expected token exception error");

        match err {
            Error::KiteApi(api_err) => {
                assert_eq!(api_err.status_code, 403);
                assert!(matches!(
                    api_err.error_type,
                    KiteApiException::TokenException
                ));
                assert_eq!(api_err.error_type.as_str(), "TokenException");
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_holdings_authorise_token_exception_error() {
        let mut server = mockito::Server::new_async().await;
        let credentials = KiteCredentials::new(
            "TEST_API_KEY",
            "TEST_API_SECRET",
            "TEST_USER_ID",
            "TEST_PASSWORD",
            "TEST_TOTP",
        );
        let config = Config::from_parts(
            server.url(),
            KITECONNECT_API_LOGIN.to_string(),
            KITECONNECT_API_REDIRECT.to_string(),
            credentials,
        );
        let client = HTTPClient::with_config(config);

        let _m = server
            .mock("POST", "/portfolio/holdings/authorise")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Token is invalid or has expired",
                    "error_type": "TokenException"
                }"#,
            )
            .create_async()
            .await;

        let portfolio_api = Portfolio::new(&client);
        let items = vec![HoldingAuthorisationItem {
            isin: "INE002A01018".to_string(),
            quantity: 50,
        }];

        let err = portfolio_api
            .authorise_holdings(&items)
            .await
            .expect_err("expected token exception error");

        match err {
            Error::KiteApi(api_err) => {
                assert_eq!(api_err.status_code, 403);
                assert!(matches!(
                    api_err.error_type,
                    KiteApiException::TokenException
                ));
                assert_eq!(api_err.error_type.as_str(), "TokenException");
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }
}
