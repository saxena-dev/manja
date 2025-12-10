//! User API group: `/user/`
//!
//! This module provides functionality to interact with the user-related
//! endpoints of the Kite Connect HTTP API.

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::Result;
use manja_core::models::{KiteApiResponse, Segment, SegmentKind, UserMargins, UserProfile};

/// User related API endpoints for fetching user margins and profile information.
pub struct User<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> User<'c> {
    /// Creates a new instance of `User` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `User` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Fetch the user profile from the API endpoint: `/user/profile`.
    pub async fn profile(&self) -> Result<KiteApiResponse<UserProfile>> {
        self.client.get(&"/user/profile", &self.backoff).await
    }

    /// Fetch the user margins from the API endpoint: `/user/margins`.
    pub async fn margins(&self) -> Result<KiteApiResponse<UserMargins>> {
        self.client.get(&"/user/margins", &self.backoff).await
    }

    /// Fetch the user margins for a specific segment (`equity` or `commodity`)
    /// from the API endpoint: `/user/margins/:segment`.
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

    use crate::test_utils::{
        add_mocks, get_http_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
    };
    use manja_core::models::{Segment, SegmentKind, UserMargins, UserProfile};

    use super::*;

    // TODO: These tests depend on user fixtures that are currently
    // missing from the updated `kiteconnect-mocks` submodule
    // (`user_profile.json`, `user_margins.json`). They are temporarily
    // commented out and should be re-enabled once the fixtures are restored.

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

    // #[tokio::test]
    // async fn test_user_profile() {
    //     let (server, client) = get_http_test_client().await;
    //     let (_server,) = tokio::join!(add_mocks(server, mock_map()));
    //
    //     let response = client.user().profile().await.unwrap();
    //     let profile =
    //         read_to_object::<UserProfile>("./kiteconnect-mocks/api_docs/user_profile.json")
    //             .unwrap();
    //     assert_eq!(response.data.unwrap(), profile);
    // }
    //
    // #[tokio::test]
    // async fn test_user_margins() {
    //     let (server, client) = get_http_test_client().await;
    //     let (_server,) = tokio::join!(add_mocks(server, mock_map()));
    //
    //     let response = client.user().margins().await.unwrap();
    //     let margins =
    //         read_to_object::<UserMargins>("./kiteconnect-mocks/api_docs/user_margins.json")
    //             .unwrap();
    //     assert_eq!(response.data.unwrap(), margins);
    // }
    //
    // #[tokio::test]
    // async fn test_user_margins_commodity_segment() {
    //     let (server, client) = get_http_test_client().await;
    //     let (_server,) = tokio::join!(add_mocks(server, mock_map()));
    //
    //     let response = client
    //         .user()
    //         .margins_by_segment(SegmentKind::Commodity)
    //         .await
    //         .unwrap();
    //     let segment =
    //         read_to_object::<Segment>("./kiteconnect-mocks/margin_commodity.json").unwrap();
    //     assert_eq!(response.data.unwrap(), segment);
    // }
    //
    // #[tokio::test]
    // async fn test_user_margins_equity_segment() {
    //     let (server, client) = get_http_test_client().await;
    //     let (_server,) = tokio::join!(add_mocks(server, mock_map()));
    //
    //     let response = client
    //         .user()
    //         .margins_by_segment(SegmentKind::Equity)
    //         .await
    //         .unwrap();
    //     let segment =
    //         read_to_object::<Segment>("./kiteconnect-mocks/margins_equity.json").unwrap();
    //     assert_eq!(response.data.unwrap(), segment);
    // }
}
