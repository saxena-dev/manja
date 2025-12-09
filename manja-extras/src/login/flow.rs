//! Kite Connect login flow using WebDriver.
//!
//! This module provides functionality to automate the login flow for Kite
//! Connect using a browser driven by WebDriver. It handles navigating to the
//! login page, entering user credentials, generating TOTP codes, and
//! retrieving the request token from the redirected URL after successful
//! authentication.

use fantoccini::Locator;
use manja_core::traits::CoreConfig;
use secrecy::ExposeSecret;
use url::Url;

use crate::login::chrome::BrowserClient;
use crate::login::chrome::launch_browser;
use crate::login::error::{LoginError, Result};
use crate::login::totp::generate_totp;

/// Performs the browser-based login flow to obtain a request token.
///
/// This asynchronous function automates the process of logging into Kite
/// Connect platform by controlling a browser. It navigates to the login page,
/// enters the user ID and password, generates a TOTP code, and retrieves the
/// request token from the redirected URL.
///
/// The configuration type is expressed in terms of the shared `manja-core`
/// traits so that higher-level crates can provide their own config
/// implementations without depending on this crate.
pub async fn browser_login_flow<C>(config: C) -> Result<String>
where
    C: CoreConfig + Send,
{
    let api_key = config.api_key();
    let user_id = config.user_id();
    let password = config.user_password();
    let totp_key = config.totp_key();

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

    match wait_for_url(&client, config.api_redirect(), tokio::time::Duration::from_secs(10))
        .await
    {
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
                None => Err(LoginError::Internal(
                    "`request_token` not found in redirect URL.".to_string(),
                )),
            }
        }
        Err(e) => Err(e),
    }
}

/// Waits for the browser to navigate to a specific URL.
///
/// This asynchronous helper function checks the browser's current URL at
/// regular intervals and compares it to the expected redirect URL. It returns
/// the current URL once the browser navigates to the expected domain.
async fn wait_for_url(
    client: &BrowserClient,
    url_base: &str,
    timeout: tokio::time::Duration,
) -> Result<Url> {
    let url_redirect = Url::parse(url_base)
        .map_err(|_| LoginError::InvalidRedirectUrl(url_base.to_string()))?;
    let start = tokio::time::Instant::now();
    while start.elapsed() < timeout {
        // Get the current URL
        match client.current_url().await {
            Ok(current_url) => {
                if current_url.domain() == url_redirect.domain() {
                    return Ok(current_url);
                }
            }
            Err(e) => return Err(LoginError::from(e)),
        }
        // Wait for a short duration before checking again
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }
    Err(LoginError::Timeout)
}
