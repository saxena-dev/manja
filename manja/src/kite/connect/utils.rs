//! Utility functions used across the [crate::kite::connect] module.
//!

use sha2::{Digest, Sha256};

/// Generates a checksum required for retrieving the user access token from Kite
/// Connect API.
///
/// The checksum is created by concatenating the API key, request token, and API
/// secret, then hashing the result using the SHA-256 algorithm, and finally
/// encoding it as a hexadecimal string.
///
/// # Arguments
///
/// * `api_key` - The API key obtained from the Kite Connect developer portal.
/// * `request_token` - The one-time token obtained after the login flow, used
///     to request the access token.
/// * `api_secret` - The API secret obtained from the Kite Connect developer portal.
///
/// # Returns
///
/// A `String` containing the SHA-256 checksum in hexadecimal format.
///
pub fn create_checksum(api_key: &str, request_token: &str, api_secret: &str) -> String {
    // SHA256 hasher instance
    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}{}", api_key, request_token, api_secret));
    // Encode to hexadecimal string
    hex::encode(hasher.finalize())
}
