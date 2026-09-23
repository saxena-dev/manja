//! The token exchange response (`kite-api-docs/docs/connect/v3/user.md:33-120`).
//!
//! [`UserSession`] holds the user profile and the tokens a successful
//! exchange returns. Every token is secret-wrapped: `Debug` redacts it, and
//! the type implements no `Serialize`, so generic serialization cannot export
//! a credential:
//!
//! ```compile_fail
//! # fn f(s: manja::kite::connect::models::UserSession) {
//! let _ = serde_json::to_string(&s);
//! # }
//! ```
//!
//! Reading a token is an explicit call, such as [`UserSession::credentials`].
//! Nothing in the SDK stores or installs the returned tokens: the caller
//! decides whether to keep them and which clients to build with them.
//!
use chrono::{DateTime, FixedOffset};
use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Deserializer};

use crate::kite::connect::credentials::{AccessToken, ApiKey, CredentialError, Credentials};
use crate::kite::protocol::datetime::parse_broker_datetime;

/// Additional session metadata.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Meta {
    /// Demat consent: empty, `consent` or `physical`.
    pub demat_consent: String,
}

/// A successful token exchange.
///
/// The access token expires at 6 AM the next day unless invalidated earlier
/// (`user.md:115`). A refresh token is issued only to certain approved
/// platforms (`user.md:117`); an empty value is `None`.
#[derive(Clone, Debug)]
pub struct UserSession {
    /// User's registered role, `individual` for retail users.
    pub user_type: String,
    /// User's email.
    pub email: String,
    /// User's real name.
    pub user_name: String,
    /// Shortened name.
    pub user_shortname: String,
    /// Broker ID.
    pub broker: String,
    /// Exchanges enabled for the user.
    pub exchanges: Vec<String>,
    /// Margin products enabled for the user.
    pub products: Vec<String>,
    /// Order types enabled for the user.
    pub order_types: Vec<String>,
    /// Avatar URL, if any.
    pub avatar_url: Option<String>,
    /// User ID.
    pub user_id: String,
    /// The API key the exchange was performed for.
    pub api_key: Secret<String>,
    /// The access token for subsequent requests.
    pub access_token: Secret<String>,
    /// Token for public session validation.
    pub public_token: Secret<String>,
    /// Refresh token, for approved platforms only.
    pub refresh_token: Option<Secret<String>>,
    /// The `enctoken`, if returned.
    pub enctoken: Option<Secret<String>>,
    /// Last login time (IST).
    pub login_time: Option<DateTime<FixedOffset>>,
    /// Additional metadata.
    pub meta: Option<Meta>,
}

impl UserSession {
    /// Build runtime [`Credentials`] from the returned API key and access
    /// token. This is the intentional export point; nothing else in the SDK
    /// reads the tokens.
    pub fn credentials(&self) -> Result<Credentials, CredentialError> {
        Ok(Credentials::from_parts(
            ApiKey::new(self.api_key.expose_secret().as_str())?,
            AccessToken::new(self.access_token.expose_secret().as_str())?,
        ))
    }
}

fn non_empty(s: Option<String>) -> Option<Secret<String>> {
    s.filter(|v| !v.is_empty()).map(Secret::new)
}

impl<'de> Deserialize<'de> for UserSession {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Fields {
            user_type: String,
            email: String,
            user_name: String,
            user_shortname: String,
            broker: String,
            exchanges: Vec<String>,
            products: Vec<String>,
            order_types: Vec<String>,
            #[serde(default)]
            avatar_url: Option<String>,
            user_id: String,
            api_key: String,
            access_token: String,
            public_token: String,
            #[serde(default)]
            refresh_token: Option<String>,
            #[serde(default)]
            enctoken: Option<String>,
            #[serde(default)]
            login_time: Option<String>,
            #[serde(default)]
            meta: Option<Meta>,
        }

        let f = Fields::deserialize(deserializer)?;
        let login_time = f
            .login_time
            .map(|t| parse_broker_datetime(&t))
            .transpose()
            .map_err(serde::de::Error::custom)?;
        Ok(UserSession {
            user_type: f.user_type,
            email: f.email,
            user_name: f.user_name,
            user_shortname: f.user_shortname,
            broker: f.broker,
            exchanges: f.exchanges,
            products: f.products,
            order_types: f.order_types,
            avatar_url: f.avatar_url,
            user_id: f.user_id,
            api_key: Secret::new(f.api_key),
            access_token: Secret::new(f.access_token),
            public_token: Secret::new(f.public_token),
            refresh_token: non_empty(f.refresh_token),
            enctoken: non_empty(f.enctoken),
            login_time,
            meta: f.meta,
        })
    }
}
