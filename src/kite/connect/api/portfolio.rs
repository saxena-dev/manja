//! Portfolio API group: `/portfolio/`
//!
//! This module provides functionality to interact with the portfolio-related
//! endpoints of Kite Connect API.
//!
//! A user's portfolio consists of long term equity holdings and short term
//! positions. The portfolio APIs return instruments in a portfolio with up-to-date
//! profit and loss computations.
//!
//! Refer to the official [API documentation](https://kite.trade/docs/connect/v3/portfolio/).
//!
use crate::kite::connect::api::{create_backoff_policy, BackoffPolicy};
use crate::kite::connect::{
    client::HTTPClient,
    models::{Auction, Holding, KiteApiResponse, Position, PositionConversionRequest},
};
use crate::kite::error::Result;

/// A user's portfolio consists of long term equity holdings and short term
/// positions. The portfolio APIs return instruments in a portfolio with
/// up-to-date profit and loss computations.
///
pub struct Portfolio<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Portfolio<'c> {
    /// Creates a new instance of `Portfolio` with default API rate limits.
    ///
    /// # Arguments
    ///
    /// * `client` - A reference to the `HTTPClient` used for making API requests.
    ///
    /// # Returns
    ///
    /// A new instance of `Portfolio`.
    ///
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Portfolio` instance.
    ///
    /// # Arguments
    ///
    /// * `backoff` - A `BackoffPolicy` instance specifying the backoff policy.
    ///
    /// # Returns
    ///
    /// The `Portfolio` instance with the updated backoff policy.
    ///
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Retrieve the list of long term equity holdings.
    ///
    /// Holdings contain the user's portfolio of long term equity delivery
    /// stocks. An instrument in a holdings portfolio remains there
    /// indefinitely until its sold or is delisted or changed by the exchanges.
    /// Underneath it all, instruments in the holdings reside in the user's
    /// DEMAT account, as settled by exchanges and clearing institutions.
    ///
    pub async fn get_holdings(&self) -> Result<KiteApiResponse<Vec<Holding>>> {
        self.client
            .get(&format!("/portfolio/holdings"), &self.backoff)
            .await
    }

    /// Retrieve the list of short term positions.
    ///
    /// Positions contain the user's portfolio of short to medium term derivatives
    /// (futures and options contracts) and intraday equity stocks. Instruments
    /// in the positions portfolio remain there until they're sold, or until
    /// expiry, which, for derivatives, is typically three months. Equity positions
    /// carried overnight move to the holdings portfolio the next day.
    ///
    /// The positions API returns two sets of positions, `net` and `day`. `net`
    /// is the actual, current net position portfolio, while `day` is a snapshot
    /// of the buying and selling activity for that particular day. This is
    /// useful for computing intraday profits and losses for trading strategies.
    ///
    pub async fn get_positions(&self) -> Result<KiteApiResponse<Vec<Position>>> {
        self.client
            .get(&format!("/portfolio/positions"), &self.backoff)
            .await
    }

    /// Convert the margin product of an open position.
    ///
    /// All positions held are of specific margin products such as NRML, MIS
    /// etc. A position can have one and only one margin product. These
    /// products affect how the user's margin usage and free cash values are
    /// computed, and a user may want to covert or change a position's margin
    /// product from time to time. More on [margin policies](https://zerodha.com/z-connect/general/zerodha-margin-policies).
    ///
    pub async fn convert_position(
        &self,
        request: PositionConversionRequest,
    ) -> Result<KiteApiResponse<bool>> {
        self.client
            .put(&format!("/portfolio/positions"), request, &self.backoff)
            .await
    }

    /// Retrieve the list of auctions that are currently being held.
    ///
    /// This API returns a list of auctions that are currently being held,
    /// along with details about each auction such as the auction number,
    /// the security being auctioned, the last price of the security, and
    /// the quantity of the security being offered. Only the stocks that
    /// you hold in your demat account will be shown in the auctions list.
    ///
    pub async fn get_auctions(&self) -> Result<KiteApiResponse<Vec<Auction>>> {
        self.client
            .get(&format!("/portfolio/holdings/auctions"), &self.backoff)
            .await
    }

    // TODO!
    // Initiating authorisation
    //
    // curl --request POST https://api.kite.trade/portfolio/holdings/authorise
    // -H "X-Kite-Version: 3" \
    // -H "Authorization: token api_key:access_token" \
    // -d "isin=INE002A01018" -d "quantity=50" \
    // -d "isin=INE009A01021" -d "quantity=50"
    //
    // {
    // "status": "success",
    // "data": {
    // "request_id": "na8QgCeQm05UHG6NL9sAGRzdfSF64UdB"
    // }
    // }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mockito::ServerGuard;
    use tokio::join;

    use crate::kite::connect::client::test_utils::{
        add_mocks, get_manja_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
    };
    use crate::kite::connect::client::HTTPClient;
    use crate::kite::connect::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
    use crate::kite::connect::credentials::KiteCredentials;
    use crate::kite::connect::models::{Auction, Holding};
    use crate::kite::error::{KiteApiException, ManjaError};
    use crate::test_support::init_tracing;

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(
            ("GET", "/portfolio/holdings"),
            "./kiteconnect-mocks/holdings.json",
        );
        mmap.insert(
            ("GET", "/portfolio/holdings/auctions"),
            "./kiteconnect-mocks/auctions_list.json",
        );
        mmap
    }

    #[tokio::test]
    async fn test_get_holdings_success() {
        init_tracing();
        let (server, mut manja_client) = get_manja_test_client().await;
        let server_ptr: *const ServerGuard = &server;
        log::debug!("Server @address: {:p}", server_ptr);
        let (_server,) = join!(add_mocks(server, mock_map()));

        let portfolio_api = Portfolio::new(&manja_client);
        let response = portfolio_api.get_holdings().await.unwrap();
        let expected =
            read_to_object::<Vec<Holding>>("./kiteconnect-mocks/holdings.json").unwrap();

        let holdings = response.data.expect("expected holdings data");
        assert_eq!(holdings.len(), expected.len());
        assert_eq!(holdings[0].tradingsymbol, expected[0].tradingsymbol);
    }

    #[tokio::test]
    async fn test_get_auctions_success() {
        let (server, mut manja_client) = get_manja_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let portfolio_api = Portfolio::new(&manja_client);
        let response = portfolio_api.get_auctions().await.unwrap();
        let expected =
            read_to_object::<Vec<Auction>>("./kiteconnect-mocks/auctions_list.json").unwrap();

        let auctions = response.data.expect("expected auctions data");
        assert_eq!(auctions.len(), expected.len());
        assert_eq!(auctions[0].tradingsymbol, expected[0].tradingsymbol);
    }

    #[tokio::test]
    async fn test_get_holdings_token_exception_error() {
        init_tracing();
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
        let mut client = HTTPClient::with_config(config);

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
            .get_holdings()
            .await
            .expect_err("expected token exception error");

        match err {
            ManjaError::KiteApiError(api_err) => {
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
