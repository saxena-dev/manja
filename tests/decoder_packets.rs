//! Packet families decoded field by field (plan task S22).
//!
//! Baselines: the official `ticker_quote.packet` and `ticker_full.packet`
//! (base64 text of one bare packet) with their JSON, the vendored qdx
//! single-packet and multi-packet pairs, and every packet of the real
//! capture. `ticker_ltp.json` has no packet file, so the LTP layout is
//! covered by the qdx `single_ltp` pair only. The real capture holds only
//! full and index-full packets (plan A-02), so it cannot exercise the LTP,
//! quote or index-quote layouts.

mod support;

use chrono::{DateTime, FixedOffset};
use manja::kite::decoder::framing::{frame, FramingLimits, Message};
use manja::kite::decoder::packets::{decode, decode_bytes, scaled, DepthEntry, Packet};
use manja::kite::protocol::scale::Segment;
use serde_json::{json, Map, Value};

use support::capture::{read_capture, read_real_capture, read_ticker_fixture};
use support::fixtures;

fn oracle(name: &str) -> Value {
    let path = support::capture::ticker_fixtures_dir().join(format!("protocol/{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn depth(entries: &[DepthEntry; 5]) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|e| json!({"quantity": e.quantity, "price": e.price, "orders": e.orders}))
            .collect(),
    )
}

// The packet in the oracle's field names, raw integers only.
fn as_oracle(p: &Packet, raw_len: usize) -> Value {
    let mut m = Map::new();
    m.insert("raw_len".into(), json!(raw_len));
    let mut put = |k: &str, v: Value| {
        m.insert(k.into(), v);
    };
    match p {
        Packet::Ltp(p) => {
            put("kind", json!("ltp"));
            put("instrument_token", json!(p.instrument_token.get()));
            put("last_traded_price", json!(p.last_price));
        }
        Packet::Quote(q) => {
            put("kind", json!("quote"));
            quote(&mut put, &q.fields);
        }
        Packet::Full(f) => {
            put("kind", json!("full"));
            quote(&mut put, &f.fields);
            put("last_traded_timestamp", json!(f.last_trade_time));
            put("open_interest", json!(f.open_interest));
            put("open_interest_day_high", json!(f.open_interest_day_high));
            put("open_interest_day_low", json!(f.open_interest_day_low));
            put("exchange_timestamp", json!(f.exchange_timestamp));
            put("bid_depth", depth(&f.depth.bids));
            put("ask_depth", depth(&f.depth.offers));
        }
        Packet::IndexQuote(i) => {
            put("kind", json!("index_quote"));
            index(&mut put, &i.fields);
        }
        Packet::IndexFull(i) => {
            put("kind", json!("index_full"));
            index(&mut put, &i.fields);
            put("exchange_timestamp", json!(i.exchange_timestamp));
        }
        _ => panic!("unexpected family"),
    }
    Value::Object(m)
}

fn quote(put: &mut impl FnMut(&str, Value), q: &manja::kite::decoder::packets::QuoteFields) {
    put("instrument_token", json!(q.instrument_token.get()));
    put("last_traded_price", json!(q.last_price));
    put("last_traded_quantity", json!(q.last_quantity));
    put("average_traded_price", json!(q.average_price));
    put("volume_traded_for_day", json!(q.volume));
    put("total_buy_quantity", json!(q.buy_quantity));
    put("total_sell_quantity", json!(q.sell_quantity));
    put("open_price", json!(q.open));
    put("high_price", json!(q.high));
    put("low_price", json!(q.low));
    put("close_price", json!(q.close));
}

fn index(put: &mut impl FnMut(&str, Value), i: &manja::kite::decoder::packets::IndexFields) {
    put("instrument_token", json!(i.instrument_token.get()));
    put("last_traded_price", json!(i.last_price));
    put("high_price", json!(i.high));
    put("low_price", json!(i.low));
    put("open_price", json!(i.open));
    put("close_price", json!(i.close));
    put("change", json!(i.change));
}

fn decoded(payload: &[u8]) -> Vec<Value> {
    let Message::Packets(frames) = frame(payload, FramingLimits::default()).unwrap() else {
        panic!("a batch")
    };
    frames
        .iter()
        .map(|f| as_oracle(&decode(&f).unwrap(), f.bytes().len()))
        .collect()
}

#[test]
fn every_qdx_packet_pair_matches_field_by_field() {
    for name in [
        "single_ltp",
        "single_quote",
        "single_full",
        "single_index_quote",
        "single_index_full",
    ] {
        let got = decoded(&read_ticker_fixture(&format!("protocol/{name}.bin")));
        assert_eq!(got, [oracle(name)["packet"].clone()], "{name}");
    }
    let got = decoded(&read_ticker_fixture("protocol/multi_packet.bin"));
    assert_eq!(Value::Array(got), oracle("multi_packet")["packets"]);
}

fn base64(text: &str) -> Vec<u8> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
    {
        acc = (acc << 6) | alphabet.iter().position(|a| *a == c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn price(raw: i32) -> f64 {
    scaled(raw, Segment::Nse).unwrap().to_f64()
}

fn ist(secs: u32) -> String {
    DateTime::from_timestamp(secs as i64, 0)
        .unwrap()
        .with_timezone(&FixedOffset::east_opt(19800).unwrap())
        .to_rfc3339()
}

#[test]
fn the_official_quote_and_full_samples_decode_to_their_expectations() {
    let q = base64(&fixtures::read("ticker_quote.packet").unwrap());
    assert_eq!(q.len(), 44);
    let Packet::Quote(q) = decode_bytes(&q).unwrap() else {
        panic!()
    };
    let e: Value = fixtures::json("ticker_quote.json").unwrap();
    let f = q.fields;
    assert_eq!(
        f.instrument_token.get() as u64,
        e["instrument_token"].as_u64().unwrap()
    );
    assert_eq!(price(f.last_price), e["last_price"].as_f64().unwrap());
    assert_eq!(f.last_quantity as u64, e["last_quantity"].as_u64().unwrap());
    assert_eq!(price(f.average_price), e["average_price"].as_f64().unwrap());
    assert_eq!(f.volume as u64, e["volume"].as_u64().unwrap());
    assert_eq!(f.buy_quantity as u64, e["buy_quantity"].as_u64().unwrap());
    assert_eq!(f.sell_quantity as u64, e["sell_quantity"].as_u64().unwrap());
    for (k, v) in [
        ("open", f.open),
        ("high", f.high),
        ("low", f.low),
        ("close", f.close),
    ] {
        assert_eq!(price(v), e["ohlc"][k].as_f64().unwrap(), "{k}");
    }
    // Disposition: the JSON `change` (5.9) has no field in the 44-byte
    // tradable quote packet and is not last minus close (5.35); it is not
    // derivable from the packet, so it is not asserted.
    assert!(price(f.last_price) - price(f.close) != e["change"].as_f64().unwrap());

    let full = base64(&fixtures::read("ticker_full.packet").unwrap());
    assert_eq!(full.len(), 184);
    let Packet::Full(p) = decode_bytes(&full).unwrap() else {
        panic!()
    };
    let e: Value = fixtures::json("ticker_full.json").unwrap();
    assert_eq!(
        price(p.fields.last_price),
        e["last_price"].as_f64().unwrap()
    );
    assert_eq!(p.fields.volume as u64, e["volume"].as_u64().unwrap());
    assert_eq!(
        ist(p.last_trade_time),
        e["last_trade_time"].as_str().unwrap()
    );
    assert_eq!(ist(p.exchange_timestamp), e["timestamp"].as_str().unwrap());
    assert_eq!(p.open_interest as u64, e["oi"].as_u64().unwrap());
    for (side, entries) in [("buy", p.depth.bids), ("sell", p.depth.offers)] {
        let expected = e["depth"][side].as_array().unwrap();
        assert_eq!(expected.len(), 5);
        for (got, want) in entries.iter().zip(expected) {
            assert_eq!(price(got.price), want["price"].as_f64().unwrap(), "{side}");
            assert_eq!(
                got.quantity as u64,
                want["quantity"].as_u64().unwrap(),
                "{side}"
            );
            assert_eq!(
                got.orders as u64,
                want["orders"].as_u64().unwrap(),
                "{side}"
            );
        }
    }
}

#[test]
fn every_capture_packet_decodes_and_index_packets_stay_distinct() {
    let capture = read_real_capture();
    let (mut full, mut index) = (0, 0);
    for r in read_capture(&capture).unwrap() {
        let Ok(Message::Packets(frames)) = frame(r.payload, FramingLimits::default()) else {
            continue;
        };
        for f in frames.iter() {
            match decode(&f).unwrap() {
                Packet::Full(p) => {
                    full += 1;
                    assert_eq!((p.depth.bids.len(), p.depth.offers.len()), (5, 5));
                }
                Packet::IndexFull(_) => index += 1,
                other => panic!("unexpected {other:?}"),
            }
        }
    }
    assert!(full > 0 && index > 0, "{full} {index}");
    assert_eq!(full + index, count_packets(&capture));
}

fn count_packets(capture: &[u8]) -> usize {
    read_capture(capture)
        .unwrap()
        .iter()
        .filter_map(|r| match frame(r.payload, FramingLimits::default()) {
            Ok(Message::Packets(f)) => Some(f.len()),
            _ => None,
        })
        .sum()
}
