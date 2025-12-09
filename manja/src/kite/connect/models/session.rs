//! User session type.
//!
//! This module provides structures and functions for managing user sessions
//! and authentication in Kite Connect API.
//!
use secrecy::{ExposeSecret, Secret};
use serde::{ser::SerializeStruct, Deserialize, Deserializer, Serialize, Serializer};

/// Represents additional metadata for the user session.
///
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Meta {
    /// Consent for demat account.
    demat_consent: String,
}

/// Represents a user's session, including authentication tokens and profile
/// information.
///
#[derive(Clone, Debug)]
pub struct UserSession {
    /// Type of user.
    pub user_type: String,
    /// User's email address.
    pub email: String,
    /// User's email address.
    pub user_name: String,
    /// User's short name.
    pub user_shortname: String,
    /// Broker's name.
    pub broker: String,
    /// List of exchanges enabled for the user.
    pub exchanges: Vec<String>,
    /// List of product types enabled for the user.
    pub products: Vec<String>,
    /// List of order types enabled for the user.
    pub order_types: Vec<String>,
    /// URL to the user's avatar.
    pub avatar_url: Option<String>,
    /// Unique user ID.
    pub user_id: String,
    /// API key.
    pub api_key: Secret<String>,
    /// Access token for authentication.
    pub access_token: Secret<String>,
    /// Public token for session validation.
    pub public_token: Secret<String>,
    /// Refresh token for extended access.
    pub refresh_token: Secret<String>,
    /// Encrypted token.
    pub enctoken: Secret<String>,
    /// Timestamp of the user's last login.
    pub login_time: String,
    /// Additional metadata for the session.
    pub meta: Option<Meta>,
}

// Custom implementation of `Serialize` for `UserSession` because secrets
// should not be exposed.
impl Serialize for UserSession {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("UserSession", 16)?;
        state.serialize_field("user_type", &self.user_type)?;
        state.serialize_field("email", &self.email)?;
        state.serialize_field("user_name", &self.user_name)?;
        state.serialize_field("user_shortname", &self.user_shortname)?;
        state.serialize_field("broker", &self.broker)?;
        state.serialize_field("exchanges", &self.exchanges)?;
        state.serialize_field("products", &self.products)?;
        state.serialize_field("order_types", &self.order_types)?;
        state.serialize_field("avatar_url", &self.avatar_url)?;
        state.serialize_field("user_id", &self.user_id)?;
        state.serialize_field("api_key", self.api_key.expose_secret())?;
        state.serialize_field("access_token", self.access_token.expose_secret())?;
        state.serialize_field("public_token", self.public_token.expose_secret())?;
        state.serialize_field("refresh_token", self.refresh_token.expose_secret())?;
        state.serialize_field("enctoken", self.enctoken.expose_secret())?;
        state.serialize_field("login_time", &self.login_time)?;
        state.serialize_field("meta", &self.meta)?;
        state.end()
    }
}

// Custom implementation of `Deserialize` for `UserSession`.
impl<'de> Deserialize<'de> for UserSession {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct UserSessionFields {
            user_type: String,
            email: String,
            user_name: String,
            user_shortname: String,
            broker: String,
            exchanges: Vec<String>,
            products: Vec<String>,
            order_types: Vec<String>,
            avatar_url: Option<String>,
            user_id: String,
            api_key: String,
            access_token: String,
            public_token: String,
            refresh_token: String,
            enctoken: String,
            login_time: String,
            meta: Option<Meta>,
        }

        let fields = UserSessionFields::deserialize(deserializer)?;

        Ok(UserSession {
            user_type: fields.user_type,
            email: fields.email,
            user_name: fields.user_name,
            user_shortname: fields.user_shortname,
            broker: fields.broker,
            exchanges: fields.exchanges,
            products: fields.products,
            order_types: fields.order_types,
            avatar_url: fields.avatar_url,
            user_id: fields.user_id,
            api_key: Secret::new(fields.api_key),
            access_token: Secret::new(fields.access_token),
            public_token: Secret::new(fields.public_token),
            refresh_token: Secret::new(fields.refresh_token),
            enctoken: Secret::new(fields.enctoken),
            login_time: fields.login_time,
            meta: fields.meta,
        })
    }
}
