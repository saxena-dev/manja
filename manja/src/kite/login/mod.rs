//! Automated authentication flow using user credentials.
//!
//! This module serves as the main entry point for `manja` crate's automated
//! login support. It includes submodules that provide functionality for interacting
//! with a Chrome browser for authentication, managing the authentication flow,
//! and generating Time-based One-Time Passwords (TOTP).
//!
mod chrome;
pub use chrome::launch_browser;

mod flow;
pub use flow::browser_login_flow;

mod totp;
pub use totp::generate_totp;
