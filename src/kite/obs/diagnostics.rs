//! Bounded, redacted building blocks for typed diagnostics.
//!
//! Diagnostics are available without any subscriber, recorder or exporter.
//! They are bounded in memory and history ([`FailureHistory`], `B-DIAG-01`)
//! and in text ([`BoundedText`], `B-DIAG-02`), carry revisions so a stale view
//! is detectable, and are not an audit database.
//!
//! Redaction happens before text is retained. Structurally, SDK code never
//! places credentials, request or response bodies, payloads or URLs in a
//! diagnostic. Free text that originates outside the SDK, such as a broker
//! error message, additionally passes through [`BoundedText::sanitize`],
//! which masks credential-shaped content as defense in depth.
//!
use std::collections::VecDeque;
use std::fmt;

/// Default bytes retained per diagnostic text field (`B-DIAG-02`).
pub const DEFAULT_TEXT_BYTES: usize = 512;
/// Largest permitted text bound (`B-DIAG-02`).
pub const MAX_TEXT_BYTES: usize = 4096;
/// Default failure-history length (`B-DIAG-01`).
pub const DEFAULT_HISTORY: usize = 16;
/// Largest permitted failure-history length (`B-DIAG-01`).
pub const MAX_HISTORY: usize = 256;

/// Runs of this many ASCII alphanumerics are masked as credential-shaped.
const SECRET_RUN: usize = 24;
/// `key=value` pairs whose value is always masked.
const SECRET_KEYS: &[&str] = &[
    "api_key",
    "access_token",
    "request_token",
    "refresh_token",
    "public_token",
    "enctoken",
    "api_secret",
    "checksum",
    "token",
];
const MASK: &str = "<redacted>";

/// A sanitized, length-bounded piece of text.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BoundedText {
    text: String,
    truncated: bool,
}

impl BoundedText {
    /// Sanitize `input`, then keep at most `max_bytes` bytes (capped at
    /// [`MAX_TEXT_BYTES`]) on a character boundary, marking truncation.
    ///
    /// Sanitizing masks values of `key=value` or `key: value` pairs for
    /// credential keys (`access_token`, `api_key`, `checksum`, …), the
    /// credential after a `token ` authorization scheme, and any run of
    /// 24 or more ASCII letters and digits.
    pub fn sanitize(input: &str, max_bytes: usize) -> Self {
        let masked = mask(input);
        let max = max_bytes.min(MAX_TEXT_BYTES);
        if masked.len() <= max {
            return Self {
                text: masked,
                truncated: false,
            };
        }
        let mut end = max;
        while !masked.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            text: masked[..end].to_string(),
            truncated: true,
        }
    }

    /// The retained text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether text was cut off.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

impl fmt::Debug for BoundedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.text)?;
        if self.truncated {
            f.write_str("…")?;
        }
        Ok(())
    }
}

impl fmt::Display for BoundedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)?;
        if self.truncated {
            f.write_str("…")?;
        }
        Ok(())
    }
}

fn mask(input: &str) -> String {
    // Pass 1: credential key/value pairs and `token <credential>`.
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    'outer: while !rest.is_empty() {
        for key in SECRET_KEYS {
            if let Some(tail) = strip_key(rest, key) {
                let value_len = tail
                    .find(|c: char| c.is_whitespace() || matches!(c, '&' | ',' | ';' | '"' | '\''))
                    .unwrap_or(tail.len());
                if value_len > 0 {
                    out.push_str(&rest[..rest.len() - tail.len()]);
                    out.push_str(MASK);
                    rest = &tail[value_len..];
                    continue 'outer;
                }
            }
        }
        let c = rest.chars().next().expect("non-empty");
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    // Pass 2: long alphanumeric runs.
    let mut result = String::with_capacity(out.len());
    let mut run = String::new();
    for c in out.chars() {
        if c.is_ascii_alphanumeric() {
            run.push(c);
        } else {
            flush_run(&mut result, &mut run);
            result.push(c);
        }
    }
    flush_run(&mut result, &mut run);
    result
}

// If `s` starts with `key` followed by `=`, `:`, `: ` or (for `token`) a
// space, return the text after the separator. Matching is case-insensitive
// and deliberately conservative: it also fires inside longer words.
fn strip_key<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let head = s.get(..key.len())?;
    if !head.eq_ignore_ascii_case(key) {
        return None;
    }
    let tail = &s[key.len()..];
    for sep in ["=", ": ", ":", " "] {
        if sep == " " && key != "token" {
            continue;
        }
        if let Some(v) = tail.strip_prefix(sep) {
            return Some(v);
        }
    }
    None
}

fn flush_run(out: &mut String, run: &mut String) {
    if run.len() >= SECRET_RUN {
        out.push_str(MASK);
    } else {
        out.push_str(run);
    }
    run.clear();
}

/// A bounded history of the most recent failures, oldest evicted first.
///
/// Every push increments [`Self::revision`], so a reader can tell whether a
/// snapshot is stale.
#[derive(Clone, Debug)]
pub struct FailureHistory<T> {
    entries: VecDeque<T>,
    capacity: usize,
    revision: u64,
}

impl<T: Clone> FailureHistory<T> {
    /// A history of at most `capacity` entries, clamped to
    /// `1..=`[`MAX_HISTORY`].
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.clamp(1, MAX_HISTORY);
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
            revision: 0,
        }
    }

    /// Record a failure, evicting the oldest one when full.
    pub fn push(&mut self, entry: T) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
        self.revision += 1;
    }

    /// Entries, oldest first.
    pub fn entries(&self) -> Vec<T> {
        self.entries.iter().cloned().collect()
    }

    /// Number of pushes so far.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Configured capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Kind of a decoder-adapter diagnostic (contract §6.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DecodeDiagnosticKind {
    /// The message ended inside a header or packet.
    Truncated,
    /// The declared packet count does not fit the message or its bound.
    CountMismatch,
    /// Bytes remained after the declared packets.
    TrailingBytes,
    /// A packet had a length no documented family uses.
    UnknownLength,
    /// The message exceeded its size bound.
    Oversized,
    /// A text message was not valid JSON of a supported shape.
    InvalidText,
    /// A text message had a type this build does not know.
    UnknownTextType,
}

impl DecodeDiagnosticKind {
    /// Every kind.
    pub const ALL: &'static [Self] = &[
        Self::Truncated,
        Self::CountMismatch,
        Self::TrailingBytes,
        Self::UnknownLength,
        Self::Oversized,
        Self::InvalidText,
        Self::UnknownTextType,
    ];

    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Truncated => "truncated",
            Self::CountMismatch => "count_mismatch",
            Self::TrailingBytes => "trailing_bytes",
            Self::UnknownLength => "unknown_length",
            Self::Oversized => "oversized",
            Self::InvalidText => "invalid_text",
            Self::UnknownTextType => "unknown_text_type",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: &str = "Zq9sentinelSECRETtoken00XYZ";

    #[test]
    fn credential_shapes_are_masked_before_retention() {
        for input in [
            format!("Invalid access_token={SENTINEL} supplied"),
            format!("api_key: {SENTINEL}"),
            format!("Authorization: token {SENTINEL}"),
            format!("checksum={SENTINEL}&x=1"),
            format!("raw {SENTINEL} tail"),
        ] {
            let t = BoundedText::sanitize(&input, DEFAULT_TEXT_BYTES);
            assert!(!t.as_str().contains(SENTINEL), "{input} -> {t}");
            assert!(!format!("{t:?}").contains(SENTINEL));
        }
        // Ordinary broker text survives.
        let t = BoundedText::sanitize("Order 151220000000000 rejected", 512);
        assert_eq!(t.as_str(), "Order 151220000000000 rejected");
    }

    #[test]
    fn text_is_bounded_on_a_char_boundary() {
        let t = BoundedText::sanitize(&"é".repeat(400), 11);
        assert!(t.is_truncated());
        assert!(t.as_str().len() <= 11);
        let t = BoundedText::sanitize("short", 0);
        assert_eq!(t.as_str(), "");
        let t = BoundedText::sanitize(&"x ".repeat(5000), usize::MAX);
        assert!(t.as_str().len() <= MAX_TEXT_BYTES);
    }

    #[test]
    fn retained_history_never_holds_a_seeded_secret() {
        let mut h = FailureHistory::new(4);
        h.push(BoundedText::sanitize(
            &format!("error: access_token={SENTINEL}"),
            512,
        ));
        h.push(BoundedText::sanitize(SENTINEL, 512));
        let rendered = format!("{:?}", h.entries());
        assert!(!rendered.contains(SENTINEL), "{rendered}");
    }

    #[test]
    fn history_is_bounded_with_revisions() {
        let mut h = FailureHistory::new(2);
        for i in 0..5 {
            h.push(i);
        }
        assert_eq!(h.entries(), vec![3, 4]);
        assert_eq!(h.revision(), 5);
        assert_eq!(FailureHistory::<u8>::new(0).capacity(), 1);
        assert_eq!(FailureHistory::<u8>::new(10_000).capacity(), MAX_HISTORY);
    }
}
