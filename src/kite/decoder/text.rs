//! Text messages (`kite:websocket.md:165-184`).
//!
//! A text message is a JSON object `{"type": …, "data": …}`:
//!
//! | `type` | Result |
//! |---|---|
//! | `order` | [`TextEvent::Order`]: `data` parsed as a partial [`OrderUpdate`], unknown status and enum values preserved |
//! | `error` | [`TextEvent::Error`]: the broker's error string, bounded |
//! | `message` | [`TextEvent::Message`]: the broker's message string, bounded |
//! | anything else | [`TextEvent::Unknown`]: the type name, bounded, and the `data` value as received |
//!
//! Size (`B-DEC-03`) and nesting depth are checked before JSON parsing;
//! malformed JSON, a missing or non-string `type`, a `data` of the wrong
//! shape and an order update without `order_id` are [`TextError`]s with
//! bounded detail, never panics. Unknown content is preserved but carries
//! no meaning: it is neither an instruction nor a success. A text message is
//! never a market tick, and connection lifecycle is not a text event.
//!
//! Parsing is pure: no clock, credential, telemetry, runtime or network is
//! involved. It claims no webhook authentication (the postback `checksum`
//! is not verified here) and no deduplication; those are the application's.
//!
use std::fmt;

use serde_json::Value;

use crate::kite::obs::diagnostics::{BoundedText, DecodeDiagnosticKind, DEFAULT_TEXT_BYTES};
use crate::kite::protocol::OrderUpdate;

/// Default `B-DEC-03`: text message bytes, 1 MiB.
pub const DEFAULT_MAX_TEXT: usize = 1 << 20;
/// Maximum JSON nesting depth of a text message.
pub const MAX_DEPTH: usize = 32;

/// Text parsing bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextLimits {
    max_bytes: usize,
}

impl Default for TextLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_TEXT,
        }
    }
}

impl TextLimits {
    /// Text message bytes (`B-DEC-03`, 1 KiB to 16 MiB).
    pub fn with_max_bytes(
        mut self,
        n: usize,
    ) -> Result<Self, crate::kite::decoder::framing::DecoderLimitError> {
        if !(1 << 10..=16 << 20).contains(&n) {
            return Err(crate::kite::decoder::framing::DecoderLimitError("B-DEC-03"));
        }
        self.max_bytes = n;
        Ok(self)
    }

    /// `B-DEC-03`.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

/// A parsed text message.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TextEvent {
    /// An order update.
    Order(Box<OrderUpdate>),
    /// A broker error string.
    Error(BoundedText),
    /// A broker message or alert.
    Message(BoundedText),
    /// A type this build does not know, preserved without interpretation.
    Unknown {
        /// The `type` value, bounded.
        message_type: BoundedText,
        /// The `data` value as received, if any.
        data: Option<Value>,
    },
}

/// Why a text message could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextError {
    kind: DecodeDiagnosticKind,
    detail: BoundedText,
}

impl TextError {
    fn new(kind: DecodeDiagnosticKind, detail: &str) -> Self {
        Self {
            kind,
            detail: BoundedText::sanitize(detail, DEFAULT_TEXT_BYTES),
        }
    }

    /// The diagnostic kind: `Oversized` or `InvalidText`.
    pub fn kind(&self) -> DecodeDiagnosticKind {
        self.kind
    }

    /// Bounded, sanitized detail.
    pub fn detail(&self) -> &BoundedText {
        &self.detail
    }
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "text message: {}: {}",
            self.kind.as_str(),
            self.detail.as_str()
        )
    }
}

impl std::error::Error for TextError {}

// The nesting depth of JSON text, without parsing it: brackets outside
// strings.
fn depth(text: &str) -> usize {
    let (mut depth, mut max, mut in_string, mut escaped) = (0usize, 0usize, false, false);
    for b in text.bytes() {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                max = max.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

/// Parse one text message.
pub fn parse(text: &str, limits: TextLimits) -> Result<TextEvent, TextError> {
    use DecodeDiagnosticKind::{InvalidText, Oversized};
    if text.len() > limits.max_bytes {
        return Err(TextError::new(
            Oversized,
            "the text message exceeds its bound",
        ));
    }
    if depth(text) > MAX_DEPTH {
        return Err(TextError::new(
            InvalidText,
            "the JSON nesting exceeds its bound",
        ));
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|e| TextError::new(InvalidText, &format!("not JSON: {e}")))?;
    let Value::Object(mut object) = value else {
        return Err(TextError::new(
            InvalidText,
            "the message is not a JSON object",
        ));
    };
    let message_type = match object.get("type") {
        Some(Value::String(t)) => t.clone(),
        _ => {
            return Err(TextError::new(
                InvalidText,
                "the message has no string type",
            ))
        }
    };
    let data = object.remove("data");
    let string = |data: Option<Value>| match data {
        Some(Value::String(s)) => Ok(BoundedText::sanitize(&s, DEFAULT_TEXT_BYTES)),
        _ => Err(TextError::new(InvalidText, "the data is not a string")),
    };
    match message_type.as_str() {
        "order" => {
            let Some(data @ Value::Object(_)) = data else {
                return Err(TextError::new(InvalidText, "order data is not an object"));
            };
            let update: OrderUpdate = serde_json::from_value(data).map_err(|e| {
                TextError::new(
                    InvalidText,
                    &format!("order data is not an order update: {e}"),
                )
            })?;
            Ok(TextEvent::Order(Box::new(update)))
        }
        "error" => string(data).map(TextEvent::Error),
        "message" => string(data).map(TextEvent::Message),
        _ => Ok(TextEvent::Unknown {
            message_type: BoundedText::sanitize(&message_type, DEFAULT_TEXT_BYTES),
            data,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_ignores_brackets_inside_strings() {
        assert_eq!(depth(r#"{"a":"[[[{{{","b":[1,[2]]}"#), 3);
        assert_eq!(depth(r#"{"a":"\"[","b":{}}"#), 2);
    }

    #[test]
    fn nesting_at_and_past_the_bound() {
        let at = format!(
            "{{\"type\":\"x\",\"data\":{}{}}}",
            "[".repeat(MAX_DEPTH - 1),
            "]".repeat(MAX_DEPTH - 1)
        );
        assert!(parse(&at, TextLimits::default()).is_ok());
        let past = format!(
            "{{\"type\":\"x\",\"data\":{}{}}}",
            "[".repeat(MAX_DEPTH),
            "]".repeat(MAX_DEPTH)
        );
        assert_eq!(
            parse(&past, TextLimits::default()).unwrap_err().kind(),
            DecodeDiagnosticKind::InvalidText
        );
        // Deep enough to overflow a naive recursive parser.
        let bomb = "[".repeat(100_000);
        assert!(parse(&bomb, TextLimits::default()).is_err());
    }
}
