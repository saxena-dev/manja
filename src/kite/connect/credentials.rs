//! Credential types for Kite Connect.
//!
//! [`Credentials`] is the immutable runtime snapshot that ordinary
//! authenticated HTTP requests and the ticker use. It holds exactly an
//! [`ApiKey`] and an [`AccessToken`]: no API secret, password, TOTP key,
//! browser setting or lifecycle callback. [`ApiSecret`] and [`RequestToken`]
//! exist only to be borrowed by the explicit token-exchange operation, and are
//! never stored in a snapshot.
//!
//! Every type in this module redacts its secret material in `Debug`, and none
//! of them implements `Serialize`, so generic serialization can never export a
//! credential. Reading a secret back is an explicit call to an `expose_*`
//! method.
//!
//! Nothing in this module reads environment variables, secret stores or
//! files, and nothing here performs a network request: constructing, cloning
//! and dropping a snapshot never exchanges, refreshes or invalidates a
//! session. Cloning a snapshot copies it; it does not create shared revocation.
//! Replacing credentials means constructing a new snapshot and a new client.
//!
//! ```
//! use manja::kite::connect::credentials::Credentials;
//!
//! let creds = Credentials::new("my_api_key", "my_access_token").unwrap();
//! assert_eq!(creds.api_key().as_str(), "my_api_key");
//! // Debug output never contains the token.
//! assert!(!format!("{creds:?}").contains("my_access_token"));
//! // Header construction is explicit and fallible at construction time.
//! assert_eq!(
//!     creds.authorization_header().expose(),
//!     "token my_api_key:my_access_token"
//! );
//! // Invalid input is a configuration error, not a panic.
//! assert!(Credentials::new("key", "bad\ntoken").is_err());
//! ```
//!
use std::fmt;

use secrecy::{ExposeSecret, Secret};

/// Placeholder used wherever a secret would otherwise be rendered.
pub(crate) const REDACTED: &str = "<redacted>";

/// Why a credential value was rejected. The rejected value is never included.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CredentialError {
    field: &'static str,
    kind: CredentialErrorKind,
}

/// Category of a [`CredentialError`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialErrorKind {
    /// The value is empty.
    Empty,
    /// The value contains whitespace, a control character, a non-ASCII
    /// character, or (for an API key) a `:` that would make the
    /// `Authorization` header ambiguous.
    InvalidCharacter,
}

impl CredentialError {
    fn new(field: &'static str, kind: CredentialErrorKind) -> Self {
        Self { field, kind }
    }

    /// Name of the rejected field, for example `"access_token"`.
    pub fn field(&self) -> &'static str {
        self.field
    }

    /// Why the field was rejected.
    pub fn kind(&self) -> CredentialErrorKind {
        self.kind
    }
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match self.kind {
            CredentialErrorKind::Empty => "is empty",
            CredentialErrorKind::InvalidCharacter => "contains an invalid character",
        };
        write!(f, "credential field `{}` {why}", self.field)
    }
}

impl std::error::Error for CredentialError {}

// Tokens and keys are printable ASCII without whitespace. Checking this at
// construction makes every later header or query construction infallible.
fn validate(field: &'static str, value: &str, allow_colon: bool) -> Result<(), CredentialError> {
    if value.is_empty() {
        return Err(CredentialError::new(field, CredentialErrorKind::Empty));
    }
    let ok = value
        .bytes()
        .all(|b| b.is_ascii_graphic() && (allow_colon || b != b':'));
    if ok {
        Ok(())
    } else {
        Err(CredentialError::new(
            field,
            CredentialErrorKind::InvalidCharacter,
        ))
    }
}

/// A secret string that can only be read through [`Self::expose`].
///
/// `Debug` prints `<redacted>`. Used for values built from credentials, such
/// as the `Authorization` header and the WebSocket query string.
#[derive(Clone)]
pub struct SecretText(Secret<String>);

impl SecretText {
    /// Read the secret value. Callers must not log or persist it.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

/// The public Kite Connect API key of an app.
///
/// The key is not a secret in the protocol sense, but it identifies an
/// account's app, so `Debug` redacts it and it never becomes a metric label.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Validate and wrap an API key.
    pub fn new(value: impl Into<String>) -> Result<Self, CredentialError> {
        let value = value.into();
        validate("api_key", &value, false)?;
        Ok(Self(value))
    }

    /// The key as sent on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey({REDACTED})")
    }
}

macro_rules! secret_type {
    ($(#[$doc:meta])* $name:ident, $field:literal, $expose:ident) => {
        $(#[$doc])*
        #[derive(Clone)]
        pub struct $name(Secret<String>);

        impl $name {
            /// Validate and wrap the secret value.
            pub fn new(value: impl Into<String>) -> Result<Self, CredentialError> {
                let value = value.into();
                validate($field, &value, true)?;
                Ok(Self(Secret::new(value)))
            }

            /// Read the secret value. This is the only way to obtain it;
            /// callers must not log or persist it.
            pub fn $expose(&self) -> &str {
                self.0.expose_secret()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), REDACTED)
            }
        }
    };
}

secret_type!(
    /// A Kite Connect access token, obtained from token exchange.
    AccessToken,
    "access_token",
    expose_secret
);

secret_type!(
    /// A Kite Connect API secret.
    ///
    /// Only the explicit token-exchange operation borrows it, to compute the
    /// checksum. It is never stored in a [`Credentials`] snapshot, a client or
    /// a ticker.
    ApiSecret,
    "api_secret",
    expose_secret
);

secret_type!(
    /// The one-time request token returned by the browser login redirect.
    RequestToken,
    "request_token",
    expose_secret
);

/// Immutable runtime credentials: an API key and an access token.
///
/// This is the only credential material ordinary HTTP requests and the ticker
/// hold. Construction validates both values, so building the `Authorization`
/// header or the WebSocket query string never fails afterwards.
///
/// The snapshot is immutable. Cloning it copies the values; a clone is not
/// revoked when the original is dropped, and invalidating the session at the
/// broker does not change either copy. To use new credentials, build a new
/// snapshot and a new client.
///
/// `Credentials` implements neither `Serialize` nor `Deserialize`:
///
/// ```compile_fail
/// let creds = manja::kite::connect::credentials::Credentials::new("k", "t").unwrap();
/// let _ = serde_json::to_string(&creds);
/// ```
#[derive(Clone)]
pub struct Credentials {
    api_key: ApiKey,
    access_token: AccessToken,
}

impl Credentials {
    /// Validate and build a snapshot from an API key and an access token.
    pub fn new(
        api_key: impl Into<String>,
        access_token: impl Into<String>,
    ) -> Result<Self, CredentialError> {
        Ok(Self::from_parts(
            ApiKey::new(api_key)?,
            AccessToken::new(access_token)?,
        ))
    }

    /// Build a snapshot from already validated parts.
    pub fn from_parts(api_key: ApiKey, access_token: AccessToken) -> Self {
        Self {
            api_key,
            access_token,
        }
    }

    /// The API key.
    pub fn api_key(&self) -> &ApiKey {
        &self.api_key
    }

    /// The access token. Reading its value requires
    /// [`AccessToken::expose_secret`].
    pub fn access_token(&self) -> &AccessToken {
        &self.access_token
    }

    /// The `Authorization` header value, `token api_key:access_token`
    /// (`kite-api-docs/docs/connect/v3/user.md:124`).
    pub fn authorization_header(&self) -> SecretText {
        SecretText(Secret::new(format!(
            "token {}:{}",
            self.api_key.as_str(),
            self.access_token.expose_secret()
        )))
    }

    /// The WebSocket connection query string,
    /// `api_key=…&access_token=…` (`kite-api-docs/docs/connect/v3/websocket.md:20`).
    ///
    /// Both values are validated printable ASCII; the reserved query
    /// characters `&`, `=`, `#`, `+`, `%` and `?` are percent-encoded.
    pub fn websocket_query(&self) -> SecretText {
        SecretText(Secret::new(format!(
            "api_key={}&access_token={}",
            encode_query_value(self.api_key.as_str()),
            encode_query_value(self.access_token.expose_secret())
        )))
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("api_key", &self.api_key)
            .field("access_token", &self.access_token)
            .finish()
    }
}

// Percent-encode the characters that are significant inside a query value.
pub(crate) fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'&' | b'=' | b'#' | b'+' | b'%' | b'?' | b' ' => {
                out.push_str(&format!("%{b:02X}"));
            }
            _ => out.push(b as char),
        }
    }
    out
}

#[cfg(test)]
mod test {
    use super::*;

    const SENTINEL: &str = "SENTINEL-s3cr3t-7f1c";

    #[test]
    fn snapshot_holds_key_and_token_only() {
        let creds = Credentials::new("key", SENTINEL).unwrap();
        assert_eq!(creds.api_key().as_str(), "key");
        assert_eq!(creds.access_token().expose_secret(), SENTINEL);
        // A clone is an independent copy.
        let clone = creds.clone();
        drop(creds);
        assert_eq!(clone.access_token().expose_secret(), SENTINEL);
    }

    #[test]
    fn debug_and_errors_never_render_secrets() {
        let creds = Credentials::new("key", SENTINEL).unwrap();
        let renders = [
            format!("{creds:?}"),
            format!("{creds:#?}"),
            format!("{:?}", creds.access_token()),
            format!("{:?}", creds.authorization_header()),
            format!("{:?}", creds.websocket_query()),
            format!("{:?}", ApiSecret::new(SENTINEL).unwrap()),
            format!("{:?}", RequestToken::new(SENTINEL).unwrap()),
            // A nested value keeps the redaction.
            format!("{:?}", Some(vec![creds.clone()])),
        ];
        for render in renders {
            assert!(!render.contains(SENTINEL), "{render}");
        }
        let err = AccessToken::new(format!("{SENTINEL}\n")).unwrap_err();
        assert!(!err.to_string().contains(SENTINEL));
        assert!(!format!("{err:?}").contains(SENTINEL));
    }

    #[test]
    fn construction_is_fallible_and_explains_the_field() {
        let err = Credentials::new("", "token").unwrap_err();
        assert_eq!(
            (err.field(), err.kind()),
            ("api_key", CredentialErrorKind::Empty)
        );
        let err = Credentials::new("key", "tok en").unwrap_err();
        assert_eq!(
            (err.field(), err.kind()),
            ("access_token", CredentialErrorKind::InvalidCharacter)
        );
        // A colon in the key would make `token key:token` ambiguous.
        assert!(ApiKey::new("a:b").is_err());
        assert!(AccessToken::new("a:b").is_ok());
        assert!(AccessToken::new("tøken").is_err());
        assert!(AccessToken::new("line\r\nbreak").is_err());
    }

    #[test]
    fn header_and_query_are_built_from_the_snapshot() {
        let creds = Credentials::new("key", "a&b=c").unwrap();
        assert_eq!(creds.authorization_header().expose(), "token key:a&b=c");
        assert_eq!(
            creds.websocket_query().expose(),
            "api_key=key&access_token=a%26b%3Dc"
        );
    }

    #[test]
    fn construction_reads_no_environment() {
        // The legacy `load_from_env` path is gone: a snapshot contains only
        // what the caller passed, whatever the process environment holds.
        std::env::set_var("KITECONNECT_API_KEY", SENTINEL);
        let creds = Credentials::new("explicit", "token").unwrap();
        assert_eq!(creds.api_key().as_str(), "explicit");
    }
}
