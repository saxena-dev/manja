//! Kite Connect login flow.
//!
//! This module provides functionality to automate the login flow for Kite Connect
//! using a headless browser. It handles navigating to the login page, entering
//! user credentials, generating TOTP codes, and retrieving the request token from
//! the redirected URL after successful authentication.
//!
//! For detailed information, refer to the official Kite Connect API
//! [documentation](https://kite.trade/docs/connect/v3/user/#login-flow).
//!
use fantoccini::Locator;
use secrecy::ExposeSecret;
use url::Url;

use crate::kite::error::{ManjaError, Result};
use crate::kite::login::{
    chrome::launch_browser, tokio_sleep, totp::generate_totp, BrowserClient, TokioDuration,
};
use crate::kite::traits::KiteConfig;

/// Performs the browser-based login flow to obtain a request token.
///
/// This asynchronous function automates the process of logging into Kite Connect
/// platform by controlling a headless browser. It navigates to the login page, enters
/// the user ID and password, generates a TOTP code, and retrieves the request token
/// from the redirected URL.
///
/// # Arguments
///
/// * `config` - A boxed trait object that implements the `KiteConfig` trait,
///   providing the necessary configuration and credentials.
///
/// # Returns
///
/// A `Result` containing the request token as a `String` if successful, or a `ManjaError`
/// if the login process fails.
///
/// # Example
///
/// ```ignore
/// use manja::kite::connect::config::Config;
/// use manja::kite::login::browser_login_flow;
///
/// let config = Config::default();
/// let request_token = browser_login_flow(Box::new(config)).await?;
/// ```
///
pub async fn browser_login_flow(config: Box<dyn KiteConfig>) -> Result<String> {
    let api_key = config.credentials().api_key();
    let user_id = config.credentials().user_id();
    let password = config.credentials().user_pwd();
    let totp_key = config.credentials().totp_key();

    // Launch the browser and WebDriver process
    let (client, mut driver) = launch_browser().await?;

    // Navigate to the Zerodha login page
    let _ = client
        .goto(&format!(
            "{}?api_key={}",
            config.api_login(),
            api_key.expose_secret().as_str()
        ))
        .await;

    // Enter login ID
    client
        .wait()
        .for_element(Locator::XPath(r#"//*[@id="userid"]"#))
        .await?
        .send_keys(user_id.expose_secret().as_str())
        .await?;

    // Enter password
    client
        .wait()
        .for_element(Locator::XPath(r#"//*[@id="password"]"#))
        .await?
        .send_keys(password.expose_secret().as_str())
        .await?;

    // Click the login button
    client
        .wait()
        .for_element(Locator::XPath(
            r#"//*[@id="container"]/div/div/div[2]/form/div[4]/button"#,
        ))
        .await?
        .click()
        .await?;

    // Generate the TOTP code for the current time
    let current_code = generate_totp(totp_key.expose_secret().as_str());

    // Enter TOTP access token
    client
        .wait()
        .for_element(Locator::XPath(r#"//*[@label="External TOTP"]"#))
        .await?
        .send_keys(&current_code)
        .await?;

    match wait_for_url(&client, config.api_redirect(), TokioDuration::from_secs(10)).await {
        Ok(url_token) => {
            match url_token
                .query_pairs()
                .find(|(key, _)| key == "request_token")
                .map(|(_, value)| value.to_string())
            {
                Some(request_token) => {
                    client.close().await.unwrap();
                    driver.kill().await.unwrap();
                    Ok(request_token)
                }
                None => Err(ManjaError::Internal(format!(
                    "`request_token` not found in redirect URL."
                ))),
            }
        }
        Err(e) => Err(e),
    }
}

/// Waits for the browser to navigate to a specific URL.
///
/// This asynchronous helper function checks the browser's current URL at regular
/// intervals and compares it to the expected redirect URL. It returns the current
/// URL once the browser navigates to the expected domain.
///
/// # Arguments
///
/// * `client` - A reference to the browser client controlling the headless browser.
/// * `url_base` - The base URL to wait for.
/// * `timeout` - The maximum duration to wait for the redirect.
///
/// # Returns
///
/// A `Result` containing the `Url` if the browser navigates to the expected URL
/// within the timeout period, or a `ManjaError` if it times out or encounters an error.
///
async fn wait_for_url(
    client: &BrowserClient,
    url_base: &str,
    timeout: TokioDuration,
) -> Result<Url> {
    let url_redirect = Url::parse(url_base).map_err(|_| {
        ManjaError::Internal(format!("cannot parse Kite redirect url - `{}`", url_base))
    })?;
    let start = tokio::time::Instant::now();
    while start.elapsed() < timeout {
        // Get the current URL
        match client.current_url().await {
            Ok(current_url) => {
                if current_url.domain() == url_redirect.domain() {
                    return Ok(current_url);
                }
            }
            Err(e) => return Err(ManjaError::from(e)),
        }
        // Wait for a short duration before checking again
        tokio_sleep(TokioDuration::from_millis(200)).await;
    }
    Err("Timed out waiting for redirect URL".into())
}
