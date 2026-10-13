//! The constraints the synthetic-derived corpus re-imposes, checked on it
//! (`docs/verification.md` §1.2).
//!
//! The corpus is a perturbation of a private capture that keeps its shape
//! and its instrument tokens, changes every other non-zero value, and then
//! re-imposes these constraints. The checks that need
//! the private original (no non-zero value other than a token equals its
//! original, the zero mask is unchanged, the same seed
//! reproduces the same bytes) run where the original is kept; these run here,
//! with manja's own decoder, so they also document what the vendored corpus
//! guarantees. The perturbation's choices are mirrored exactly: zero fields
//! mean "absent" and are skipped; the day's range brackets the previous close
//! as well as open and last price; and an index `change` of zero is kept as
//! the exchange sent it, so the identity is checked only where it is non-zero.

// Shared with other test crates, which use helpers this one does not.
#[allow(dead_code)]
#[path = "support/capture.rs"]
mod capture;

use std::collections::BTreeMap;

use manja::kite::decoder::framing::{FramingLimits, Message, frame};
use manja::kite::decoder::packets::{Packet, decode};

/// Every decoded packet of the corpus, in file order, with its record index.
fn packets() -> Vec<(usize, Packet)> {
    let bytes = capture::read_synthetic_corpus();
    let records = capture::read_capture(&bytes).unwrap();
    let mut out = Vec::new();
    for (i, record) in records.iter().enumerate() {
        match frame(record.payload, FramingLimits::default()).unwrap() {
            Message::Packets(frames) => {
                for f in frames.iter() {
                    out.push((i, decode(&f).unwrap()));
                }
            }
            Message::Heartbeat => {}
        }
    }
    out
}

/// `low <= v <= high` for every non-zero `v`, where a zero bound is absent.
fn bracketed(low: i32, high: i32, inside: &[i32]) -> bool {
    inside
        .iter()
        .filter(|&&v| v != 0)
        .all(|&v| (low == 0 || low <= v) && (high == 0 || v <= high))
}

#[test]
fn the_corpus_has_the_vendored_shape() {
    let packets = packets();
    assert_eq!(packets.len(), 2_692);
    let full = packets
        .iter()
        .filter(|(_, p)| matches!(p, Packet::Full(_)))
        .count();
    let index = packets
        .iter()
        .filter(|(_, p)| matches!(p, Packet::IndexFull(_)))
        .count();
    assert_eq!((full, index), (2_399, 293));
}

#[test]
fn receipt_times_strictly_increase() {
    let bytes = capture::read_synthetic_corpus();
    let records = capture::read_capture(&bytes).unwrap();
    assert_eq!(records.len(), 19);
    assert!(
        records
            .windows(2)
            .all(|w| w[0].receipt_unix_nanos < w[1].receipt_unix_nanos)
    );
}

#[test]
fn the_days_range_brackets_open_close_and_last_price() {
    let mut checked = 0;
    for (i, p) in packets() {
        let (open, high, low, close, last) = match p {
            Packet::Full(f) => (
                f.fields.open,
                f.fields.high,
                f.fields.low,
                f.fields.close,
                f.fields.last_price,
            ),
            Packet::IndexFull(x) => (
                x.fields.open,
                x.fields.high,
                x.fields.low,
                x.fields.close,
                x.fields.last_price,
            ),
            _ => continue,
        };
        assert!(
            bracketed(low, high, &[open, close, last]),
            "record {i} token {}: low {low} high {high} open {open} close {close} last {last}",
            p.instrument_token().get()
        );
        checked += 1;
    }
    assert_eq!(checked, 2_692);
}

#[test]
fn depth_is_ordered_and_the_best_offer_is_above_the_best_bid() {
    let mut books = 0;
    for (i, p) in packets() {
        let Packet::Full(f) = p else { continue };
        let bids: Vec<i32> = f
            .depth
            .bids
            .iter()
            .map(|e| e.price)
            .filter(|&v| v != 0)
            .collect();
        let offers: Vec<i32> = f
            .depth
            .offers
            .iter()
            .map(|e| e.price)
            .filter(|&v| v != 0)
            .collect();
        assert!(
            bids.windows(2).all(|w| w[0] > w[1]),
            "record {i}: bids {bids:?}"
        );
        assert!(
            offers.windows(2).all(|w| w[0] < w[1]),
            "record {i}: offers {offers:?}"
        );
        if let (Some(b), Some(o)) = (bids.first(), offers.first()) {
            assert!(o > b, "record {i}: best bid {b} best offer {o}");
            books += 1;
        }
    }
    // The capture this corpus derives from was taken after the close, when a
    // full packet carries one side of the book at most; the ordering checks
    // above still run on every side, and this records how many two-sided
    // books there were to compare.
    assert_eq!(books, 0);
}

#[test]
fn cumulative_volume_never_decreases_per_instrument() {
    let mut last: BTreeMap<u32, u32> = BTreeMap::new();
    let mut checked = 0;
    for (i, p) in packets() {
        let Packet::Full(f) = p else { continue };
        let volume = f.fields.volume;
        if volume == 0 {
            continue;
        }
        let token = f.fields.instrument_token.get();
        if let Some(prev) = last.insert(token, volume) {
            assert!(
                volume >= prev,
                "record {i} token {token}: {prev} then {volume}"
            );
            checked += 1;
        }
    }
    // In this post-close corpus every instrument's full packet appears once, so
    // there is no second value to compare; the check guards any corpus that
    // repeats an instrument, and this pins that it found none to compare here.
    assert_eq!(checked, 0);
    assert_eq!(last.len(), 2_399);
}

#[test]
fn index_change_is_last_price_minus_close_where_sent() {
    let (mut checked, mut zero) = (0, 0);
    for (i, p) in packets() {
        let Packet::IndexFull(x) = p else { continue };
        let f = x.fields;
        if f.change == 0 {
            zero += 1;
            continue;
        }
        assert_eq!(
            f.change,
            f.last_price - f.close,
            "record {i} token {}",
            f.instrument_token.get()
        );
        checked += 1;
    }
    assert!(checked > 0);
    assert_eq!(checked + zero, 293);
}

#[test]
fn the_last_trade_is_never_after_the_exchange_time() {
    let mut checked = 0;
    for (i, p) in packets() {
        let Packet::Full(f) = p else { continue };
        if f.last_trade_time != 0 && f.exchange_timestamp != 0 {
            assert!(
                f.last_trade_time <= f.exchange_timestamp,
                "record {i}: {} after {}",
                f.last_trade_time,
                f.exchange_timestamp
            );
            checked += 1;
        }
    }
    assert!(checked > 0);
}

/// Every numeric field of a packet, in layout order; the token is excluded.
fn fields(p: &Packet) -> Vec<i64> {
    match p {
        Packet::Full(f) => {
            let q = f.fields;
            let mut v: Vec<i64> = vec![
                q.last_price.into(),
                q.last_quantity.into(),
                q.average_price.into(),
                q.volume.into(),
                q.buy_quantity.into(),
                q.sell_quantity.into(),
                q.open.into(),
                q.high.into(),
                q.low.into(),
                q.close.into(),
                f.last_trade_time.into(),
                f.open_interest.into(),
                f.open_interest_day_high.into(),
                f.open_interest_day_low.into(),
                f.exchange_timestamp.into(),
            ];
            for e in f.depth.bids.iter().chain(f.depth.offers.iter()) {
                v.extend([
                    i64::from(e.quantity),
                    i64::from(e.price),
                    i64::from(e.orders),
                ]);
            }
            v
        }
        Packet::IndexFull(x) => {
            let f = x.fields;
            vec![
                f.last_price.into(),
                f.high.into(),
                f.low.into(),
                f.open.into(),
                f.close.into(),
                f.change.into(),
                x.exchange_timestamp.into(),
            ]
        }
        _ => Vec::new(),
    }
}

#[test]
fn the_zero_fields_are_pinned() {
    // A zero means "absent" (one side of a closed book, an index without
    // depth) and the perturbation never changes one, so the count of zero
    // fields is part of the corpus's shape. Equality of the zero mask with
    // the original is checked where the original is kept; this pins the
    // vendored counts over the same 110 006 numeric fields.
    let (mut full, mut index) = ((0, 0), (0, 0));
    for (_, p) in packets() {
        let v = fields(&p);
        let zeros = v.iter().filter(|&&x| x == 0).count();
        match p {
            Packet::Full(_) => full = (full.0 + zeros, full.1 + v.len()),
            Packet::IndexFull(_) => index = (index.0 + zeros, index.1 + v.len()),
            _ => {}
        }
    }
    assert_eq!(
        full,
        (78_293, 107_955),
        "zero and total fields of full packets"
    );
    assert_eq!(index, (56, 2_051), "zero and total fields of index packets");
}
