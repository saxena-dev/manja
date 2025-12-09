//! Chrome and WebDriver launcher.
//!
//! This module provides functionality for launching and interacting with a
//! Chrome browser instance using WebDriver. It includes a function to launch
//! the browser and establish a connection with the WebDriver, facilitating
//! browser automation tasks.
//!
//! # Environment Variables
//!
//! The following environment variables must be set to use the functionality
//! in this module:
//!
//! - `CHROME_BINARY_PATH`: The path to the Chrome browser binary.
//! - `CHROMEDRIVER_PATH`: The path to the Chromedriver executable.
//!
use crate::kite::error::Result;
use crate::kite::login::BrowserClient;

use serde_json::Map;
use std::env;

type WebDriverProcess = tokio::process::Child;

/// Launches a Chrome browser instance using WebDriver.
///
/// This function reads the paths for the Chrome binary and Chromedriver from
/// the environment variables `CHROME_BINARY_PATH` and `CHROMEDRIVER_PATH`
/// respectively. It then starts the Chromedriver on port 9515 and connects to it.
///
/// # Returns
///
/// A tuple containing:
/// - `BrowserClient`: The client to interact with the browser.
/// - `WebDriverProcess`: The `tokio` child process handle for the Chromedriver.
///
/// # Errors
///
/// This function will return an error if:
/// - The environment variables `CHROME_BINARY_PATH` or `CHROMEDRIVER_PATH` are not set.
/// - The Chromedriver fails to start.
/// - The connection to the WebDriver fails.
///
/// # Example
///
/// ```ignore
/// use manja::kite::login::launch_browser;
///
/// let (client, driver_process) = launch_browser().await.unwrap();
/// // Use the client to interact with the browser
/// // ...
/// // Don't forget to terminate the driver process when done
/// driver_process.kill().await.unwrap();
/// ```
///
pub async fn launch_browser() -> Result<(BrowserClient, WebDriverProcess)> {
    let chrome_binary_path = env::var("CHROME_BINARY_PATH")?;
    let chromedriver_path = env::var("CHROMEDRIVER_PATH")?;
    let port = 9515;

    // Start chromedriver on a specific port
    let driver = tokio::process::Command::new(chromedriver_path)
        .arg(format!("--port={}", port))
        .spawn()
        .expect("Failed to start chromedriver");

    // Allow some time for chromedriver to start up
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Set up capabilities specific to Chrome
    let mut caps = Map::new();
    caps.insert(
        "goog:chromeOptions".to_string(),
        serde_json::json!({
            "binary": chrome_binary_path,
            "args": ["--disable-gpu", "--window-size=1280,800"]
            // "args": ["--headless", "--disable-gpu", "--window-size=1280,800"]
        }),
    );

    let driver_url = format!("http://localhost:{}", port);

    // Connect to the WebDriver
    match fantoccini::ClientBuilder::native()
        .capabilities(caps)
        .connect(&driver_url)
        .await
    {
        Ok(client) => Ok((client, driver)),
        Err(e) => Err(e.into()),
    }
}
