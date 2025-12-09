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

