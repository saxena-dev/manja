//! Generate a TOTP (Time-based One-Time Password) compliant with RFC4648.
//!
//! This module provides utility functions for time and TOTP (Time-based One-Time
//! Password) generation using the RFC4648 standard. It includes functions to get
//! the current Unix epoch time, decode a Base32 encoded string, and generate a
//! TOTP token.
//!
use crate::kite::error::{ManjaError, Result};
use std::time::{SystemTime, UNIX_EPOCH};

/// Retrieves the current Unix epoch time in seconds.
///
/// This function fetches the current system time and converts it to the number
/// of seconds since the Unix epoch (January 1, 1970).
///
/// # Returns
///
/// A `u64` value representing the current Unix epoch time in seconds.
///
/// # Panics
///
/// This function will panic if the system time is before the Unix epoch.
///
fn epoch_time() -> u64 {
    // Get the current system time
    let now = SystemTime::now();
    // Convert to seconds since the Unix epoch.
    now.duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
}

/// Decodes a Base32 encoded string into bytes.
///
/// This function converts the input Base32 encoded string to uppercase (to handle case insensitivity),
/// and then decodes it into a vector of bytes using the RFC4648 alphabet without padding.
///
/// # Arguments
///
/// * `secret` - A string slice containing the Base32 encoded secret.
///
/// # Returns
///
/// A `Result` containing a `Vec<u8>` with the decoded bytes, or an error if decoding fails.
///
/// # Errors
///
/// This function will return an error if the Base32 decoding fails.
///
fn decode_base32(secret: &str) -> Result<Vec<u8>> {
    // Convert the input string to uppercase to handle case insensitivity
    let upper_secret = secret.to_uppercase();
    // Decode the base32 string
    base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &upper_secret)
        .ok_or_else(|| ManjaError::TotpError(String::from("Failed to decode base32")))
}

/// Generates an RFC4648 compliant TOTP token using a private key.
///
/// The private key is decoded from a Base32 string and used with HMAC-SHA1 to
/// encode the number of seconds since the Unix epoch (epoch time counter). A
/// token is then extracted from this generated 160-bit HMAC.
///
/// # Arguments
///
/// * `totp_key` - A string slice containing the Base32 encoded TOTP key.
///
/// # Returns
///
/// A `String` containing the generated TOTP token.
///
/// # Panics
///
/// This function will panic if decoding the Base32 string or creating the TOTP instance fails.
///
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
