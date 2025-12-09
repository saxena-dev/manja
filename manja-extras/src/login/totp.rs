//! Generate a TOTP (Time-based One-Time Password) compliant with RFC4648.
//!
//! This module provides utility functions for time and TOTP (Time-based
//! One-Time Password) generation using the RFC4648 standard.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::login::error::{LoginError, Result};

/// Retrieves the current Unix epoch time in seconds.
fn epoch_time() -> u64 {
    // Get the current system time
    let now = SystemTime::now();
    // Convert to seconds since the Unix epoch.
    now.duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
}

/// Decodes a Base32 encoded string into bytes.
fn decode_base32(secret: &str) -> Result<Vec<u8>> {
    // Convert the input string to uppercase to handle case insensitivity
    let upper_secret = secret.to_uppercase();
    // Decode the base32 string
    base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &upper_secret)
        .ok_or_else(|| LoginError::TotpError(String::from("Failed to decode base32")))
}

/// Generates an RFC4648 compliant TOTP token using a private key.
///
/// The private key is decoded from a Base32 string and used with HMAC-SHA1 to
/// encode the number of seconds since the Unix epoch (epoch time counter). A
/// token is then extracted from this generated 160-bit HMAC.
pub fn generate_totp(totp_key: &str) -> String {
    let secret = decode_base32(totp_key).expect("Error while decoding base32 `totp_key`.");
    // Create a TOTP instance
    let totp_miner = totp_rs::TOTP::new(
        // Default algorithm
        totp_rs::Algorithm::SHA1,
        // Default digit length
        6,
        // Default step size
        1,
        // Default time step (interval)
        30,
        // Base32 decoded bytes
        secret,
    )
    .expect("Error while creating `totp_miner`.");
    // Generate the TOTP code for the current time
    totp_miner.generate(epoch_time())
}

