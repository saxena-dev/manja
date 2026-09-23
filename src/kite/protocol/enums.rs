//! Unknown-preserving inbound enum values.
//!
//! Broker responses may carry a status, type or product string that this
//! build does not know. [`Inbound`] keeps such a value as
//! [`Inbound::Unknown`] with its original text (bounded to
//! [`UnknownValue::MAX_BYTES`]) instead of failing the whole record or
//! coercing it into a recognized variant.
//!
//! Unknown inbound values can never become valid outbound commands: request
//! types take the plain enum `T`, and the only way from `Inbound<T>` to `T`
//! is [`Inbound::known`] or `TryFrom`, both of which refuse `Unknown`.
//!
//! ```
//! use manja::kite::connect::models::OrderStatus;
//! use manja::kite::protocol::Inbound;
//!
//! let s: Inbound<OrderStatus> = serde_json::from_str("\"COMPLETE\"").unwrap();
//! assert_eq!(s.known(), Some(&OrderStatus::Complete));
//!
//! let s: Inbound<OrderStatus> = serde_json::from_str("\"NEW STATUS\"").unwrap();
//! assert_eq!(s.as_wire(), "NEW STATUS");
//! assert!(OrderStatus::try_from(s).is_err());
//! ```
//!
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An enum with a fixed set of broker wire strings.
pub trait WireEnum: Sized + Clone + fmt::Debug + PartialEq {
    /// The known value for `s`, if any. Matching is exact.
    fn from_wire(s: &str) -> Option<Self>;
    /// The wire string of this value.
    fn as_wire(&self) -> &'static str;
}

/// An inbound broker string this build does not recognize.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct UnknownValue {
    raw: String,
    truncated: bool,
}

impl UnknownValue {
    /// Longest original text retained, in bytes.
    pub const MAX_BYTES: usize = 128;

    pub(crate) fn new(raw: &str) -> Self {
        if raw.len() <= Self::MAX_BYTES {
            return Self {
                raw: raw.to_string(),
                truncated: false,
            };
        }
        let mut end = Self::MAX_BYTES;
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            raw: raw[..end].to_string(),
            truncated: true,
        }
    }

    /// The original text, possibly truncated.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Whether the original text was longer than [`Self::MAX_BYTES`].
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

impl fmt::Debug for UnknownValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unknown({:?}", self.raw)?;
        if self.truncated {
            f.write_str(", truncated")?;
        }
        f.write_str(")")
    }
}

/// A known enum value, or the original text of an unknown one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Inbound<T> {
    /// A value this build recognizes.
    Known(T),
    /// A value this build does not recognize, preserved.
    Unknown(UnknownValue),
}

impl<T: WireEnum> Inbound<T> {
    /// Classify a wire string.
    pub fn from_wire(s: &str) -> Self {
        match T::from_wire(s) {
            Some(v) => Self::Known(v),
            None => Self::Unknown(UnknownValue::new(s)),
        }
    }

    /// The known value, or `None` for an unknown one.
    pub fn known(&self) -> Option<&T> {
        match self {
            Self::Known(v) => Some(v),
            Self::Unknown(_) => None,
        }
    }

    /// Whether the value is unknown.
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// The wire text: the known value's string or the preserved original.
    pub fn as_wire(&self) -> &str {
        match self {
            Self::Known(v) => v.as_wire(),
            Self::Unknown(u) => u.raw(),
        }
    }
}

impl<T: WireEnum> From<T> for Inbound<T> {
    fn from(v: T) -> Self {
        Self::Known(v)
    }
}

impl<T: WireEnum> fmt::Display for Inbound<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl<T: WireEnum> Serialize for Inbound<T> {
    // Echoes the value as received; this is response data, never a command.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_wire())
    }
}

impl<'de, T: WireEnum> Deserialize<'de> for Inbound<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Self::from_wire(&s))
    }
}

/// Implement [`WireEnum`], strict `Serialize`/`Deserialize`, `Display` and
/// `TryFrom<Inbound<T>>` for a unit-variant enum from one wire table.
macro_rules! wire_enum {
    ($ty:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        impl $crate::kite::protocol::WireEnum for $ty {
            fn from_wire(s: &str) -> ::std::option::Option<Self> {
                match s {
                    $($wire => Some(Self::$variant),)+
                    _ => None,
                }
            }

            fn as_wire(&self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }
        }

        impl ::std::fmt::Display for $ty {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str($crate::kite::protocol::WireEnum::as_wire(self))
            }
        }

        impl ::serde::Serialize for $ty {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> ::std::result::Result<S::Ok, S::Error> {
                s.serialize_str($crate::kite::protocol::WireEnum::as_wire(self))
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $ty {
            // Strict: unknown text is an error here. Response DTOs use
            // `Inbound<Self>` to preserve unknown values instead.
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> ::std::result::Result<Self, D::Error> {
                let s = <::std::string::String as ::serde::Deserialize>::deserialize(d)?;
                <Self as $crate::kite::protocol::WireEnum>::from_wire(&s).ok_or_else(|| {
                    ::serde::de::Error::custom(concat!("unknown ", stringify!($ty), " value"))
                })
            }
        }

        impl TryFrom<$crate::kite::protocol::Inbound<$ty>> for $ty {
            type Error = $crate::kite::protocol::UnknownValue;

            fn try_from(v: $crate::kite::protocol::Inbound<$ty>) -> ::std::result::Result<Self, Self::Error> {
                match v {
                    $crate::kite::protocol::Inbound::Known(v) => Ok(v),
                    $crate::kite::protocol::Inbound::Unknown(u) => Err(u),
                }
            }
        }
    };
}
pub(crate) use wire_enum;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Colour {
        Red,
        Blue,
    }
    wire_enum!(Colour { Red => "RED", Blue => "BLUE" });

    #[test]
    fn known_and_unknown_values_round_trip() {
        let k: Inbound<Colour> = serde_json::from_str("\"RED\"").unwrap();
        assert_eq!(k, Inbound::Known(Colour::Red));
        let u: Inbound<Colour> = serde_json::from_str("\"GREEN\"").unwrap();
        assert!(u.is_unknown());
        assert_eq!(serde_json::to_string(&u).unwrap(), "\"GREEN\"");
        assert_eq!(serde_json::to_string(&k).unwrap(), "\"RED\"");
    }

    #[test]
    fn unknown_values_cannot_become_outbound_values() {
        let u: Inbound<Colour> = Inbound::from_wire("GREEN");
        assert!(u.known().is_none());
        let refused = Colour::try_from(u).unwrap_err();
        assert_eq!(refused.raw(), "GREEN");
        // The strict enum refuses unknown text outright.
        assert!(serde_json::from_str::<Colour>("\"GREEN\"").is_err());
        // Matching is exact: no case folding.
        assert!(Inbound::<Colour>::from_wire("red").is_unknown());
    }

    #[test]
    fn unknown_text_is_bounded_on_a_char_boundary() {
        let long = "é".repeat(100);
        let u = UnknownValue::new(&long);
        assert!(u.is_truncated());
        assert!(u.raw().len() <= UnknownValue::MAX_BYTES);
        assert!(u.raw().chars().all(|c| c == 'é'));
    }
}
