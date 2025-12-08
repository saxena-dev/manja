//! User API group: `/user/`
//!
//! This module provides functionality to interact with the user-related endpoints
//! of Kite Connect API. It allows fetching user profile information and margin
//! details for various segments.
//!
//! Refer to the official API [documentation](https://kite.trade/docs/connect/v3/user/#user).
//!
use crate::kite::connect::api::{create_backoff_policy, BackoffPolicy};
use crate::kite::connect::{
    client::HTTPClient,
    models::{KiteApiResponse, Segment, SegmentKind, UserMargins, UserProfile},
};
use crate::kite::error::Result;

/// User related API endpoints for fetching user margins and profile information.
///
/// This struct provides methods to interact with the user-related API endpoints
/// of Kite Connect. It allows fetching user profile information and user margins.
///
/// Refer to the official API [documentation](https://kite.trade/docs/connect/v3/user/#user) for more details.
///
pub struct User<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> User<'c> {
    /// Creates a new instance of `User` with default API rate limits.
    ///
    /// # Arguments
    ///
    /// * `client` - A reference to the `HTTPClient` used for making API requests.
    ///
    /// # Returns
    ///
    /// A new instance of `User`.
    ///
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `User` instance.
    ///
    /// # Arguments
    ///
    /// * `backoff` - A `BackoffPolicy` instance specifying the backoff policy.
    ///
    /// # Returns
    ///
    /// The `User` instance with the updated backoff policy.
    ///
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Fetch the user profile from the API endpoint: `/user/profile`.
    ///
    /// This method retrieves the user's profile information.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with `UserProfile` data on success,
    /// or an error if the request fails.
    ///
    /// Refer to the Kite API [documentation](https://kite.trade/docs/connect/v3/user/#user-profile) for more details.
    ///
    pub async fn profile(&self) -> Result<KiteApiResponse<UserProfile>> {
        self.client.get(&"/user/profile", &self.backoff).await
    }

    /// Fetch the user margins from the API endpoint: `/user/margins`.
    ///
    /// This method retrieves the user's margins information.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with `UserMargins` data on success,
    /// or an error if the request fails.
    ///
    /// Refer to the Kite API [documentation](https://kite.trade/docs/connect/v3/user/#funds-and-margins) for more details.
    ///
    pub async fn margins(&self) -> Result<KiteApiResponse<UserMargins>> {
        self.client.get(&"/user/margins", &self.backoff).await
    }

    /// Fetch the user margins for a specific segment (`equity` or `commodity`)
    /// from the API endpoint: `/user/margins/:segment`.
    ///
    /// This method retrieves the user's margins information for a specified segment.
    ///
    /// # Arguments
    ///
    /// * `segment` - The segment kind (`equity` or `commodity`).
    ///
    /// # Returns
    ///
    /// A `Result` containing a `KiteApiResponse` with `Segment` data on success,
    /// or an error if the request fails.
    ///
    /// Refer to the Kite API [documentation](https://kite.trade/docs/connect/v3/user/#funds-and-margins) for more details.
    ///
    pub async fn margins_by_segment(
        &self,
        segment: SegmentKind,
    ) -> Result<KiteApiResponse<Segment>> {
        self.client
            .get(
                &format!("/user/margins/{}", segment.as_ref()),
                &self.backoff,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mockito::ServerGuard;
    use tokio::join;

    use crate::test_support::init_tracing;
    use crate::kite::connect::client::test_utils::{
        add_mocks, get_manja_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
    };

    use super::*;

    // TODO: This feels hacky. Fix it.
    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(
            ("GET", "/user/profile"),
            "./kiteconnect-mocks/api_docs/user_profile.json",
        );
        mmap.insert(
            ("GET", "/user/margins"),
            "./kiteconnect-mocks/api_docs/user_margins.json",
        );
        mmap.insert(
            ("GET", "/user/margins/commodity"),
            "./kiteconnect-mocks/margin_commodity.json",
        );
        mmap.insert(
            ("GET", "/user/margins/equity"),
            "./kiteconnect-mocks/margins_equity.json",
        );
        mmap
    }

    #[tokio::test]
    async fn test_user_profile() {
        init_tracing();
        let (server, manja_client) = get_manja_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = manja_client.user().profile().await.unwrap();
        let profile =
            read_to_object::<UserProfile>("./kiteconnect-mocks/api_docs/user_profile.json")
                .unwrap();
        log::debug!("Profile object: {:?}", profile);
        assert_eq!(response.data.unwrap(), profile);
    }

    #[tokio::test]
    async fn test_user_margins() {
        let (server, manja_client) = get_manja_test_client().await;
        let server_ptr: *const ServerGuard = &server;
        log::debug!("Server @address: {:p}", server_ptr);
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = manja_client.user().margins().await.unwrap();
        let margins =
            read_to_object::<UserMargins>("./kiteconnect-mocks/api_docs/user_margins.json")
                .unwrap();
        log::debug!("Margins object: {:?}", margins);
        assert_eq!(response.data.unwrap(), margins);
    }

    #[tokio::test]
    async fn test_user_margins_commodity_segment() {
        let (server, manja_client) = get_manja_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = manja_client
            .user()
            .margins_by_segment(SegmentKind::Commodity)
            .await
            .unwrap();
        let segment =
            read_to_object::<Segment>("./kiteconnect-mocks/margin_commodity.json").unwrap();
        log::debug!("Segment object: {:?}", segment);
        assert_eq!(response.data.unwrap(), segment);
    }

    #[tokio::test]
    async fn test_user_margins_equity_segment() {
        let (server, manja_client) = get_manja_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let response = manja_client
            .user()
            .margins_by_segment(SegmentKind::Equity)
            .await
            .unwrap();
        let segment = read_to_object::<Segment>("./kiteconnect-mocks/margins_equity.json").unwrap();
        log::debug!("Segment object: {:?}", segment);
        assert_eq!(response.data.unwrap(), segment);
    }
}
