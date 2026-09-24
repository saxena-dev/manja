//! Checked broker identifiers.
//!
use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Kite instrument token: an unsigned 32-bit numeric identifier
/// (`kite:postbacks.md:71`,
/// `kite:websocket.md:93`).
///
/// A token identifies an instrument only on the day it is used: exchanges may
/// reuse tokens for different derivative instruments after expiry
/// (`kite:market-quotes.md:56`), so a token is not a stable
/// historical identity, and this type claims no ownership of an instrument
/// registry. A token alone establishes neither tradability nor the exchange
/// segment.
///
/// It deserializes from a JSON number or a string of decimal digits, because
/// both forms occur in broker responses, and rejects anything outside
/// `0..=u32::MAX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstrumentToken(u32);

impl InstrumentToken {
    /// Wrap a raw token.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The raw token.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for InstrumentToken {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<InstrumentToken> for u32 {
    fn from(value: InstrumentToken) -> Self {
        value.0
    }
}

impl TryFrom<i64> for InstrumentToken {
    type Error = std::num::TryFromIntError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        u32::try_from(value).map(Self)
    }
}

impl std::str::FromStr for InstrumentToken {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u32>().map(Self)
    }
}

impl fmt::Display for InstrumentToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for InstrumentToken {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(self.0)
    }
}

impl<'de> Deserialize<'de> for InstrumentToken {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct TokenVisitor;

        impl Visitor<'_> for TokenVisitor {
            type Value = InstrumentToken;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an instrument token (u32, as a number or decimal string)")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                u32::try_from(v)
                    .map(InstrumentToken)
                    .map_err(|_| E::custom("instrument token out of u32 range"))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                InstrumentToken::try_from(v)
                    .map_err(|_| E::custom("instrument token out of u32 range"))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse()
                    .map_err(|_| E::custom("instrument token string is not a u32"))
            }
        }

        d.deserialize_any(TokenVisitor)
    }
}

/// Why a string is not a valid order ID. The message names the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderIdError {
    field: &'static str,
    rule: &'static str,
}

impl OrderIdError {
    /// The field the ID was for.
    pub fn field(&self) -> &'static str {
        self.field
    }
}

impl fmt::Display for OrderIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid `{}`: must be {}", self.field, self.rule)
    }
}

impl std::error::Error for OrderIdError {}

// A string identifier that is valid by construction: `$check` decides which
// bytes are allowed, and every way in (`new`, `TryFrom`, `FromStr`,
// `Deserialize`) runs it.
macro_rules! checked_id {
    ($(#[$doc:meta])* $name:ident, $field:literal, $rule:literal, $check:expr_2021) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap an ID.
            pub fn new(value: impl Into<String>) -> Result<Self, OrderIdError> {
                let value = value.into();
                let allowed: fn(u8) -> bool = $check;
                if value.is_empty() || value.len() > 64 || !value.bytes().all(allowed) {
                    return Err(OrderIdError {
                        field: $field,
                        rule: $rule,
                    });
                }
                Ok(Self(value))
            }

            /// The ID as sent and received.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<&str> for $name {
            type Error = OrderIdError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = OrderIdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl std::str::FromStr for $name {
            type Err = OrderIdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Self::new(String::deserialize(d)?).map_err(de::Error::custom)
            }
        }
    };
}

checked_id!(
    /// An equity order ID: 1 to 64 ASCII letters or digits.
    ///
    /// Every order ID in the official samples is a string of digits, such as
    /// `151220000000000`. The ID becomes a path segment of the order
    /// endpoints, so a value that could change the path (`/`, `..`, `?`, a
    /// space, anything outside ASCII) cannot be constructed, and a response
    /// carrying one is a decode error rather than an ID.
    ///
    /// Comparison and ordering are those of the text, so `"10"` sorts before
    /// `"9"`: order IDs work as map keys but are not a time or numeric order.
    ///
    /// ```
    /// use manja::kite::protocol::OrderId;
    ///
    /// let id: OrderId = "151220000000000".parse().unwrap();
    /// assert_eq!(id, "151220000000000");
    /// assert!(OrderId::new("1/../2").is_err());
    /// ```
    OrderId,
    "order_id",
    "1-64 ASCII letters or digits",
    |b| b.is_ascii_alphanumeric()
);

checked_id!(
    /// A mutual fund order ID: 1 to 64 ASCII letters, digits or hyphens.
    ///
    /// Comparison and ordering are those of the text.
    ///
    /// The documented mutual fund order IDs are UUIDs
    /// (`kite:mutual-funds.md:42`), so they need the hyphen that an equity
    /// [`OrderId`] rejects. The two are separate types and cannot be swapped:
    ///
    /// ```compile_fail,E0308
    /// use manja::kite::protocol::{MfOrderId, OrderId};
    ///
    /// let mf: MfOrderId = "2b6ad4b7-c84e-4c76-b459-f3a8994184f1".parse().unwrap();
    /// let equity: OrderId = mf;
    /// ```
    MfOrderId,
    "order_id",
    "1-64 ASCII letters, digits or hyphens",
    |b| b.is_ascii_alphanumeric() || b == b'-'
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_ids_accept_broker_ids_and_reject_path_changing_values() {
        for ok in [
            "151220000000000",
            "1953341975595868160",
            "A1",
            &"9".repeat(64),
        ] {
            assert_eq!(OrderId::new(ok).unwrap().as_str(), ok);
        }
        for bad in [
            "",
            "1/2",
            "..",
            "1?x=y",
            "1-2",
            "1 2",
            "१२३",
            &"9".repeat(65),
        ] {
            let e = OrderId::new(bad).unwrap_err();
            assert_eq!(e.field(), "order_id", "{bad:?}");
            assert!(e.to_string().contains("`order_id`"), "{e}");
        }
        let id: OrderId = "171229000724687".parse().unwrap();
        assert_eq!(id.to_string(), "171229000724687");
        assert_eq!(OrderId::try_from(String::from("1")).unwrap(), "1");
        assert_eq!(serde_json::to_string(&id).unwrap(), r#""171229000724687""#);
        let back: OrderId = serde_json::from_str(r#""171229000724687""#).unwrap();
        assert_eq!(back, id);
        for bad in [r#""1/../2""#, r#""""#, "171229000724687", "null"] {
            assert!(serde_json::from_str::<OrderId>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn mutual_fund_order_ids_allow_hyphens_only() {
        let id = MfOrderId::new("2b6ad4b7-c84e-4c76-b459-f3a8994184f1").unwrap();
        assert_eq!(id, "2b6ad4b7-c84e-4c76-b459-f3a8994184f1");
        assert!(
            OrderId::new(id.as_str()).is_err(),
            "an equity ID has no hyphen"
        );
        for bad in ["", "a/b", "a?b", "a b", "a_b", &"x".repeat(65)] {
            assert_eq!(
                MfOrderId::new(bad).unwrap_err().field(),
                "order_id",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn accepts_numbers_and_digit_strings_in_range() {
        let t: InstrumentToken = serde_json::from_str("408065").unwrap();
        assert_eq!(t.get(), 408065);
        let t: InstrumentToken = serde_json::from_str("\"779521\"").unwrap();
        assert_eq!(t.get(), 779521);
        assert_eq!(serde_json::to_string(&t).unwrap(), "779521");
    }

    #[test]
    fn rejects_out_of_range_and_non_numeric_values() {
        for bad in ["-1", "4294967296", "\"12a\"", "1.5", "null"] {
            assert!(
                serde_json::from_str::<InstrumentToken>(bad).is_err(),
                "{bad}"
            );
        }
        assert!(InstrumentToken::try_from(-5i64).is_err());
    }
}
