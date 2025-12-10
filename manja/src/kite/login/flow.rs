//! Kite Connect login flow facade.
//!
//! This module keeps the existing `browser_login_flow` entrypoint under
//! `manja::kite::login` while delegating the actual WebDriver/TOTP runtime to
//! the `manja-extras` crate.

use secrecy::Secret;

use crate::kite::error::Result;
use crate::kite::traits::KiteConfig;

/// Performs the browser-based login flow to obtain a request token using the
/// underlying implementation from `manja-extras`.
///
/// The signature is preserved to minimize impact on existing callers.
pub async fn browser_login_flow(config: Box<dyn KiteConfig>) -> Result<String> {
    struct CoreConfigAdapter {
        inner: Box<dyn KiteConfig>,
    }

    impl manja_core::traits::CoreApiEndpoints for CoreConfigAdapter {
        fn api_base(&self) -> &str {
            self.inner.api_base()
        }

        fn api_login(&self) -> &str {
            self.inner.api_login()
        }

        fn api_redirect(&self) -> &str {
            self.inner.api_redirect()
        }
    }

    impl manja_core::traits::CoreCredentials for CoreConfigAdapter {
        fn api_key(&self) -> Secret<String> {
            self.inner.credentials().api_key()
        }

        fn api_secret(&self) -> Secret<String> {
            self.inner.credentials().api_secret()
        }

        fn user_id(&self) -> Secret<String> {
            self.inner.credentials().user_id()
        }

        fn user_password(&self) -> Secret<String> {
            self.inner.credentials().user_pwd()
        }

        fn totp_key(&self) -> Secret<String> {
            self.inner.credentials().totp_key()
        }
    }

    let adapter = CoreConfigAdapter { inner: config };

    manja_extras::browser_login_flow(adapter)
        .await
        .map_err(Into::into)
}
