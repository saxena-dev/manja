//! The wire form of a mutation: the exact bytes a mutation sends, inspectable
//! before the call and hashable afterwards.
//!
//! Every mutation request type renders a [`WireForm`]: the HTTP method, the
//! endpoint template, the request path, the content type and the exact
//! form-encoded body the SDK will send. The mutation call renders the same
//! form and sends its bytes unchanged, so `sha256(body as received by Kite)
//! == request.wire()?.sha256()` for every mutation. The order mutations get
//! `wire()` on their request types; the mutations that take only path
//! arguments or credentials (cancellation, GTT deletion, the session
//! operations) get constructors on `WireForm` itself.
//!
//! `Display` renders the body as Kite receives it, so a reviewer can diff it
//! against the documentation; `sha256()` hashes the body, so a journal can
//! bind a sent request to the bytes a person approved. A `WireForm` holds no
//! credential type and no header: the `Authorization` header is attached by
//! the client at send time and is never part of the form. The two session
//! forms necessarily carry the documented `request_token` and `checksum`
//! fields (`kite:user.md:97-99`) or `access_token` query
//! (`kite:user.md:295-310`); the exchange form's `Display` and `Debug` print
//! `<redacted>` for the token and the checksum, the invalidation form has no
//! body so its `Display` is empty and its `Debug` prints the query with the
//! token as `<redacted>`, and no accessor returns a form's raw bytes: they
//! leave it only through `Display`, the hash and the send.
//!
use std::fmt;

use sha2::{Digest, Sha256};

use crate::kite::connect::client::endpoint_template;
use crate::kite::connect::credentials::{AccessToken, ApiKey, ApiSecret, REDACTED, RequestToken};
use crate::kite::connect::models::{
    GttRequest, HoldingsAuthorisationRequest, ModifyOrderRequest, OrderVariety, PlaceOrderRequest,
    PositionConversionRequest, RequestError,
};
use crate::kite::connect::utils::create_checksum;
use crate::kite::error::{HttpError, HttpErrorKind, TransportStage};
use crate::kite::obs::schema::{Endpoint, Method};
use crate::kite::protocol::OrderId;

/// The content type of every non-empty mutation body
/// (`kite:response-structure.md:2`).
const FORM: &str = "application/x-www-form-urlencoded";

/// The exact wire form of one mutation: what the SDK sends, before it is
/// sent.
///
/// Obtain one from a request type's `wire()` (placement, modification,
/// conversion, holdings authorisation, GTT placement and modification) or
/// from the constructors here for the mutations that have no request type.
/// Each validates the request first, so a form always describes a request
/// the SDK would send.
///
/// ```
/// use manja::kite::connect::models::{
///     Exchange, OrderType, OrderVariety, PlaceOrderRequest, ProductType, Tag, TransactionType,
/// };
/// use manja::kite::protocol::Quantity;
///
/// let request = PlaceOrderRequest::new(
///     OrderVariety::Regular,
///     Exchange::NSE,
///     "INFY",
///     TransactionType::BUY,
///     OrderType::Market,
///     Quantity::new(1).unwrap(),
///     ProductType::CashAndCarry,
///     Tag::new("rebalance42").unwrap(),
/// );
/// let wire = request.wire().unwrap();
/// assert_eq!(wire.path(), "/orders/regular");
/// assert_eq!(
///     wire.to_string(),
///     "tradingsymbol=INFY&exchange=NSE&transaction_type=BUY&order_type=MARKET\
///      &quantity=1&product=CNC&validity=DAY&tag=rebalance42"
/// );
/// assert_eq!(wire.sha256(), request.wire().unwrap().sha256());
/// ```
pub struct WireForm {
    method: Method,
    endpoint: Endpoint,
    path: String,
    query: Vec<(&'static str, String)>,
    pairs: Vec<(&'static str, String)>,
    body: Vec<u8>,
    /// Field names whose values `Display` and `Debug` redact.
    secret_fields: &'static [&'static str],
}

impl WireForm {
    fn new(
        method: Method,
        path: String,
        query: Vec<(&'static str, String)>,
        pairs: Vec<(&'static str, String)>,
        secret_fields: &'static [&'static str],
    ) -> Self {
        let endpoint = endpoint_template(method, &path);
        let body = form_urlencode(&pairs).into_bytes();
        Self {
            method,
            endpoint,
            path,
            query,
            pairs,
            body,
            secret_fields,
        }
    }

    /// A validated form, or the `Validation` error that stops the call
    /// before anything is sent.
    fn validated(
        method: Method,
        path: String,
        validate: Result<(), RequestError>,
        pairs: Vec<(&'static str, String)>,
    ) -> Result<Self, HttpError> {
        if let Err(e) = validate {
            let endpoint = endpoint_template(method, &path);
            return Err(HttpError::new(
                HttpErrorKind::Validation,
                method,
                endpoint,
                TransportStage::NotStarted,
            )
            .with_detail(&e.to_string()));
        }
        Ok(Self::new(method, path, Vec::new(), pairs, &[]))
    }

    /// The wire form of `DELETE /orders/{variety}/{order_id}`: no body
    /// (`kite:orders.md:147-162`).
    pub fn cancel_order(variety: OrderVariety, order_id: &OrderId) -> Self {
        Self::new(
            Method::Delete,
            format!("/orders/{variety}/{order_id}"),
            Vec::new(),
            Vec::new(),
            &[],
        )
    }

    /// The wire form of `DELETE /gtt/triggers/{trigger_id}`: no body
    /// (`kite:gtt.md:397-400`).
    pub fn delete_trigger(trigger_id: u64) -> Self {
        Self::new(
            Method::Delete,
            format!("/gtt/triggers/{trigger_id}"),
            Vec::new(),
            Vec::new(),
            &[],
        )
    }

    /// The wire form of the token exchange, `POST /session/token` with
    /// `api_key`, `request_token` and `checksum` = SHA-256 hex of
    /// `api_key + request_token + api_secret` (`kite:user.md:97-99`). The
    /// secret itself is not a field. `Display` and `Debug` redact the
    /// request token and the checksum.
    pub fn session_exchange(
        api_key: &ApiKey,
        request_token: &RequestToken,
        api_secret: &ApiSecret,
    ) -> Self {
        let checksum = create_checksum(
            api_key.as_str(),
            request_token.expose_secret(),
            api_secret.expose_secret(),
        );
        Self::new(
            Method::Post,
            "/session/token".to_string(),
            Vec::new(),
            vec![
                ("api_key", api_key.as_str().to_string()),
                ("request_token", request_token.expose_secret().to_string()),
                ("checksum", checksum),
            ],
            &["request_token", "checksum"],
        )
    }

    /// The wire form of session invalidation, `DELETE /session/token` with
    /// `api_key` and `access_token` as query parameters and no body
    /// (`kite:user.md:295-310`), so `Display` is empty; `Debug` prints the
    /// query with the access token as `<redacted>`.
    pub fn session_invalidation(api_key: &ApiKey, access_token: &AccessToken) -> Self {
        Self::new(
            Method::Delete,
            "/session/token".to_string(),
            vec![
                ("api_key", api_key.as_str().to_string()),
                ("access_token", access_token.expose_secret().to_string()),
            ],
            Vec::new(),
            &["access_token"],
        )
    }

    /// The HTTP method.
    pub fn method(&self) -> Method {
        self.method
    }

    /// The endpoint template, with no dynamic segment.
    pub fn endpoint(&self) -> Endpoint {
        self.endpoint
    }

    /// The request path, segments filled in and without any query.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The `Content-Type` sent with the body: the form content type when
    /// there is a body, nothing when there is none.
    pub fn content_type(&self) -> Option<&'static str> {
        (!self.body.is_empty()).then_some(FORM)
    }

    /// Length of the body in bytes.
    pub fn body_len(&self) -> usize {
        self.body.len()
    }

    /// SHA-256 of the body, exactly the bytes sent; the empty body hashes
    /// to SHA-256 of the empty string.
    pub fn sha256(&self) -> [u8; 32] {
        Sha256::digest(&self.body).into()
    }

    /// [`Self::sha256`] as lower-case hex.
    pub fn sha256_hex(&self) -> String {
        hex::encode(self.sha256())
    }

    /// The request target: the path, plus the query when there is one.
    pub(crate) fn target(&self) -> String {
        if self.query.is_empty() {
            self.path.clone()
        } else {
            format!("{}?{}", self.path, form_urlencode(&self.query))
        }
    }

    /// The body bytes, for the send.
    pub(crate) fn into_body(self) -> Vec<u8> {
        self.body
    }

    /// `pairs` as sent, except that a secret field's value is the literal
    /// `<redacted>` marker, unencoded, as everywhere else in the SDK.
    fn redacted(&self, pairs: &[(&'static str, String)]) -> String {
        if self.secret_fields.is_empty() {
            return form_urlencode(pairs);
        }
        let mut out = String::new();
        for (i, (k, v)) in pairs.iter().enumerate() {
            if i > 0 {
                out.push('&');
            }
            if self.secret_fields.contains(k) {
                out.push_str(&form_urlencode(&[(*k, String::new())]));
                out.push_str(REDACTED);
            } else {
                out.push_str(&form_urlencode(&[(*k, v.clone())]));
            }
        }
        out
    }
}

impl fmt::Display for WireForm {
    /// The body as Kite receives it; the session forms' secret fields are
    /// redacted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.redacted(&self.pairs))
    }
}

impl fmt::Debug for WireForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WireForm")
            .field("method", &self.method.as_str())
            .field("endpoint", &self.endpoint.as_str())
            .field("path", &self.path)
            .field("query", &self.redacted(&self.query))
            .field("body", &self.redacted(&self.pairs))
            .field("sha256", &self.sha256_hex())
            .finish()
    }
}

impl PlaceOrderRequest {
    /// The wire form of `POST /orders/{variety}` for this request
    /// (`kite:orders.md:58-104`), validated first.
    pub fn wire(&self) -> Result<WireForm, HttpError> {
        WireForm::validated(
            Method::Post,
            format!("/orders/{}", self.variety),
            self.validate(),
            self.form_pairs(),
        )
    }
}

impl ModifyOrderRequest {
    /// The wire form of `PUT /orders/{variety}/{order_id}` for this request
    /// (`kite:orders.md:105-146`), validated for `variety` first.
    pub fn wire(&self, variety: OrderVariety, order_id: &OrderId) -> Result<WireForm, HttpError> {
        WireForm::validated(
            Method::Put,
            format!("/orders/{variety}/{order_id}"),
            self.validate(variety),
            self.form_pairs(),
        )
    }
}

impl PositionConversionRequest {
    /// The wire form of `PUT /portfolio/positions` for this request
    /// (`kite:portfolio.md:463-497`), validated first.
    pub fn wire(&self) -> Result<WireForm, HttpError> {
        WireForm::validated(
            Method::Put,
            "/portfolio/positions".to_string(),
            self.validate(),
            self.form_pairs(),
        )
    }
}

impl HoldingsAuthorisationRequest {
    /// The wire form of `POST /portfolio/holdings/authorise` for this
    /// request (`kite:portfolio.md:516-537`), validated first; the entire
    /// holdings are requested with no body.
    pub fn wire(&self) -> Result<WireForm, HttpError> {
        WireForm::validated(
            Method::Post,
            "/portfolio/holdings/authorise".to_string(),
            self.validate(),
            self.form_pairs(),
        )
    }
}

impl GttRequest {
    /// The wire form of `POST /gtt/triggers` for this request
    /// (`kite:gtt.md:13-78`), validated first.
    pub fn wire(&self) -> Result<WireForm, HttpError> {
        WireForm::validated(
            Method::Post,
            "/gtt/triggers".to_string(),
            self.validate(),
            self.form_pairs(),
        )
    }

    /// The wire form of `PUT /gtt/triggers/{trigger_id}` replacing
    /// `trigger_id` with this request (`kite:gtt.md:373-393`), validated
    /// first.
    pub fn wire_modification(&self, trigger_id: u64) -> Result<WireForm, HttpError> {
        WireForm::validated(
            Method::Put,
            format!("/gtt/triggers/{trigger_id}"),
            self.validate(),
            self.form_pairs(),
        )
    }
}

/// Encode form fields as `application/x-www-form-urlencoded`: ASCII letters,
/// digits and `*-._` are kept, a space becomes `+`, and every other byte is
/// percent-encoded.
pub(crate) fn form_urlencode(pairs: &[(&str, String)]) -> String {
    fn enc(out: &mut String, s: &str) {
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                    out.push(b as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
    }
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        enc(&mut out, k);
        out.push('=');
        enc(&mut out, v);
    }
    out
}
