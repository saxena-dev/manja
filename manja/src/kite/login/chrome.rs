//! Chrome and WebDriver launcher facade.
//!
//! This module exposes the `launch_browser` helper under
//! `manja::kite::login::launch_browser` while delegating the actual WebDriver
//! runtime to the `manja-extras` crate.

use crate::kite::error::Result;

/// Type alias for the WebDriver client used by the login helpers.
pub type BrowserClient = manja_extras::login::chrome::BrowserClient;

/// Type alias for the spawned WebDriver process.
pub type WebDriverProcess = manja_extras::login::chrome::WebDriverProcess;

/// Launches a Chrome browser instance using WebDriver by delegating to
/// `manja-extras`.
pub async fn launch_browser() -> Result<(BrowserClient, WebDriverProcess)> {
    manja_extras::launch_browser().await.map_err(Into::into)
}
