//! Generate a TOTP (Time-based One-Time Password) compliant with RFC4648.
//!
//! This module keeps the `generate_totp` helper available under
//! `manja::kite::login::generate_totp` while delegating the implementation to
//! the `manja-extras` crate.

/// Generates an RFC4648 compliant TOTP token using a private key by delegating
/// to the implementation in `manja-extras`.
pub fn generate_totp(totp_key: &str) -> String {
    manja_extras::generate_totp(totp_key)
}
