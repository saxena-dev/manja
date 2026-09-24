//! User API group: `/user/`
//!
//! This module provides functionality to interact with the user-related endpoints
//! of Kite Connect API. It allows fetching user profile information and margin
//! details for various segments.
//!
//! Refer to the official API documentation (`kite:user.md:131-294`).
//!

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
/// Refer to the official API documentation (`kite:user.md:131-294`) for more details.
///
pub struct User<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
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
        Self { client }
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
    /// Refer to the Kite API documentation (`kite:user.md:131-197`) for more details.
    ///
    pub async fn profile(&self) -> Result<KiteApiResponse<UserProfile>> {
        self.client.get("/user/profile").await
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
    /// Refer to the Kite API documentation (`kite:user.md:198-294`) for more details.
    ///
    pub async fn margins(&self) -> Result<KiteApiResponse<UserMargins>> {
        self.client.get("/user/margins").await
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
    /// Refer to the Kite API documentation (`kite:user.md:198-294`) for more details.
    ///
    pub async fn margins_by_segment(
        &self,
        segment: SegmentKind,
    ) -> Result<KiteApiResponse<Segment>> {
        self.client
            .get(&format!("/user/margins/{}", segment.as_ref()))
            .await
    }
}
