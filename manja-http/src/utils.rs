//! Internal utilities for the `manja-http` crate.

use sha2::{Digest, Sha256};

/// Creates a checksum required for generating a session.
///
/// The checksum is calculated as SHA256 of `{api_key}{request_token}{api_secret}`.
pub fn create_checksum(api_key: &str, api_secret: &str, request_token: &str) -> String {
    let payload = format!("{}{}{}", api_key, request_token, api_secret);
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    format!("{:x}", hasher.finalize())
}

