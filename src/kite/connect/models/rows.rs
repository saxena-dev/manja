//! Row-by-row results of a list response, for the tolerant
//! `*_with_rejections` methods.
//!
//! The default list methods are strict: one element that fails to decode
//! (a bad order ID, timestamp, price or any other field) fails the whole
//! response with a `Decode` error, so an entirely rejected list can never
//! pass for an empty one. The `*_with_rejections` methods instead return
//! every element in the broker's order, decoded or rejected in place, in a
//! [`Rows`].
//!
//! A [`Rows`] may be partial. Every row can be rejected, which leaves
//! [`Rows::items`] empty although the broker sent rows; use
//! [`Rows::is_complete`] or [`Rows::into_complete`] to tell that apart from
//! an empty list.
//!
use std::fmt;

/// Every element of a list response, in the broker's array order.
#[derive(Clone, Debug, PartialEq)]
pub struct Rows<T> {
    /// The rows, decoded or rejected, in broker order.
    pub rows: Vec<Row<T>>,
}

/// One element of a list response.
#[derive(Clone, Debug, PartialEq)]
pub enum Row<T> {
    /// The element decoded into its type.
    Decoded(T),
    /// The element did not decode.
    Rejected(RowError),
}

/// Why one element of a list response did not decode.
///
/// `Debug` prints only the index. The serde error text and the raw element
/// can carry broker values (a field's value is quoted in serde's message),
/// so they are left out of it and stay reachable through the public fields.
#[derive(Clone, PartialEq)]
pub struct RowError {
    /// The element's position in the broker's array.
    pub index: usize,
    /// The decoding error. It may quote broker values.
    pub error: String,
    /// The element as the broker sent it. It may hold any broker value.
    pub raw: serde_json::Value,
}

impl fmt::Debug for RowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowError")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl<T> Rows<T> {
    /// The decoded rows, in broker order.
    pub fn items(&self) -> impl Iterator<Item = &T> {
        self.rows.iter().filter_map(|r| match r {
            Row::Decoded(t) => Some(t),
            Row::Rejected(_) => None,
        })
    }

    /// The rejected rows, in broker order.
    pub fn rejected(&self) -> impl Iterator<Item = &RowError> {
        self.rows.iter().filter_map(|r| match r {
            Row::Rejected(e) => Some(e),
            Row::Decoded(_) => None,
        })
    }

    /// Whether every row decoded.
    pub fn is_complete(&self) -> bool {
        self.rejected().next().is_none()
    }

    /// Every decoded row, if none was rejected; otherwise `self`, still
    /// holding every decoded and rejected row.
    pub fn into_complete(self) -> Result<Vec<T>, Self> {
        if !self.is_complete() {
            return Err(self);
        }
        Ok(self
            .rows
            .into_iter()
            .filter_map(|r| match r {
                Row::Decoded(t) => Some(t),
                Row::Rejected(_) => None,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejected(index: usize) -> Row<u32> {
        Row::Rejected(RowError {
            index,
            error: "invalid type: string \"SEEDvalue\", expected u32".into(),
            raw: serde_json::json!({"order_id": "SEEDvalue"}),
        })
    }

    #[test]
    fn rows_keep_broker_order_and_tell_partial_from_empty() {
        let rows = Rows {
            rows: vec![Row::Decoded(1), rejected(1), Row::Decoded(3)],
        };
        assert_eq!(rows.items().copied().collect::<Vec<_>>(), [1, 3]);
        assert_eq!(rows.rejected().map(|e| e.index).collect::<Vec<_>>(), [1]);
        assert!(!rows.is_complete());
        let back = rows.clone().into_complete().unwrap_err();
        assert_eq!(back, rows, "every row is still there");

        let all_rejected = Rows {
            rows: vec![rejected(0), rejected(1)],
        };
        assert_eq!(all_rejected.items().count(), 0);
        assert!(!all_rejected.is_complete(), "not the same as an empty list");

        let complete = Rows {
            rows: vec![Row::Decoded(7), Row::Decoded(8)],
        };
        assert_eq!(complete.into_complete().unwrap(), [7, 8]);
        assert!(Rows::<u32> { rows: vec![] }
            .into_complete()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn row_error_debug_shows_only_the_index() {
        let Row::Rejected(e) = rejected(4) else {
            unreachable!()
        };
        let debug = format!("{e:?}");
        assert!(debug.contains("index: 4"), "{debug}");
        assert!(!debug.contains("SEEDvalue"), "{debug}");
        assert!(!debug.contains("expected"), "{debug}");
        // The detail stays reachable.
        assert!(e.error.contains("SEEDvalue"));
        assert_eq!(e.raw["order_id"], "SEEDvalue");
    }
}
