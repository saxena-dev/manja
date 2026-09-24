//! The required session protocol operations: token exchange and session
//! invalidation (`kite:user.md:3-31,33-120,295-310`).
//!
//! A [`Session`] is built from an API key and an HTTP client alone. The
//! client needs no credentials: exchange is a pre-session operation.
//!
//! - [`Session::exchange`] borrows the API secret for the call only, sends
//!   `api_key`, `request_token` and `checksum = SHA-256(api_key +
//!   request_token + api_secret)` form-encoded to `POST /session/token`, and
//!   returns the typed, secret-wrapped [`UserSession`]. The secret itself is
//!   never sent and never stored.
//! - [`Session::invalidate`] sends `DELETE /session/token` with the resource's
//!   API key and the explicitly supplied access token as query parameters,
//!   as documented. It takes no API secret.
//!
//! Neither operation attaches the client's own `Authorization` header, so a
//! credential-bound client never lends an unrelated access token. Each makes
//! exactly one transport attempt under the session deadline (`B-HTTP-11`):
//! a request token is single-use, so a lost response, a 429 or a timeout is
//! reported with its stage and never retried, and never turned into assumed
//! success. Dropping the future after dispatch cancels nothing at the broker.
//!
//! Neither operation persists credentials, installs tokens into any client,
//! starts a login or coordinates credential generations; the SDK has no
//! login flow: the request token comes from the caller; invalidation does
//! not log the user out of Kite's web or mobile apps (`kite:user.md:297`), and
//! local clients keep their snapshots until the caller retires them. Nothing
//! in the SDK calls these operations implicitly: not ordinary requests, not
//! the ticker's reconnects, not shutdown and not `Drop`.
//!
use secrecy::Secret;

use crate::kite::connect::{
    client::HTTPClient,
    credentials::{AccessToken, ApiKey, ApiSecret, RequestToken},
    models::{KiteApiResponse, UserSession},
    utils::create_checksum,
};
use crate::kite::error::Result;

/// Turning a login into a session, and ending one. Borrowed from a client
/// with [`HTTPClient::session`](crate::kite::connect::client::HTTPClient::session),
/// which takes the API key.
///
/// After the user logs in on Kite's login page, Kite redirects to your app
/// with a request token. [`Session::exchange`] trades it, with your API
/// secret, for a [`UserSession`], whose [`credentials`](UserSession::credentials)
/// serve every other call. The access token stays valid until it is
/// invalidated or until 6 AM the next day (`kite:user.md:115`). Neither call
/// needs the client to hold credentials, and neither is retried.
///
/// # Example
///
/// ```no_run
/// use manja::kite::connect::client::HTTPClient;
/// use manja::kite::connect::config::Config;
/// use manja::kite::connect::credentials::{ApiKey, ApiSecret, RequestToken};
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let client = HTTPClient::new(Config::default())?;
/// let session = client
///     .session(ApiKey::new("api_key")?)
///     .exchange(&RequestToken::new("request_token")?, &ApiSecret::new("api_secret")?)
///     .await?
///     .data
///     .expect("data");
/// let credentials = session.credentials()?;
///
/// // At the end of the day, retire the access token.
/// client
///     .session(ApiKey::new("api_key")?)
///     .invalidate(credentials.access_token())
///     .await?;
/// # Ok(()) }
/// ```
pub struct Session<'c> {
    client: &'c HTTPClient,
    api_key: ApiKey,
}

impl<'c> Session<'c> {
    /// The session resource for `api_key` on `client`. The client may be
    /// unauthenticated.
    pub fn new(client: &'c HTTPClient, api_key: ApiKey) -> Self {
        Self { client, api_key }
    }

    /// The API key.
    pub fn api_key(&self) -> &ApiKey {
        &self.api_key
    }

    /// Exchange a request token for an access token: one attempt.
    ///
    /// The API secret is borrowed for the checksum only.
    pub async fn exchange(
        &self,
        request_token: &RequestToken,
        api_secret: &ApiSecret,
    ) -> Result<KiteApiResponse<UserSession>> {
        // Operation-scoped: dropped (and zeroized) when this call ends.
        let checksum = Secret::new(create_checksum(
            self.api_key.as_str(),
            request_token.expose_secret(),
            api_secret.expose_secret(),
        ));
        use secrecy::ExposeSecret;
        let pairs = vec![
            ("api_key", self.api_key.as_str().to_string()),
            ("request_token", request_token.expose_secret().to_string()),
            ("checksum", checksum.expose_secret().clone()),
        ];
        self.client
            .send_form_unauthenticated(reqwest::Method::POST, "/session/token", pairs)
            .await
    }

    /// Invalidate the session of `access_token` for this API key: one
    /// attempt. The result is the broker's; local clients are unchanged.
    ///
    /// It accepts no API secret:
    ///
    /// ```compile_fail
    /// # use manja::kite::connect::credentials::*;
    /// # async fn f(s: manja::kite::connect::api::Session<'_>, t: AccessToken, x: ApiSecret) {
    /// s.invalidate(&t, &x).await;
    /// # }
    /// ```
    pub async fn invalidate(&self, access_token: &AccessToken) -> Result<KiteApiResponse<bool>> {
        let query = [
            ("api_key", self.api_key.as_str()),
            ("access_token", access_token.expose_secret()),
        ];
        self.client
            .delete_unauthenticated("/session/token", &query)
            .await
    }
}
