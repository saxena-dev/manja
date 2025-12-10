//! WebDriver-assisted login helpers.
//!
//! This module provides optional helpers for driving a browser via WebDriver
//! to complete the Kite Connect login flow and extract the `request_token`
//! from the redirect URL. It also exposes utilities for generating TOTP codes
//! used during the second factor step.
//!
//! The public functions in this module are intended to be re-exported by the
//! `manja` facade crate under `manja::kite::login` when the
//! `webdriver-login` feature is enabled.

pub mod chrome;
mod error;
mod flow;
mod totp;

pub use chrome::launch_browser;
pub use error::{LoginError, Result};
pub use flow::browser_login_flow;
pub use totp::generate_totp;
