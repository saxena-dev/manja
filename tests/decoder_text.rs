//! Text messages without coercion.
//!
//! Baseline: the official `postback.json`, unchanged, as the `data` of a
//! `type: order` message (`kite:websocket.md:167-184`).
//! The error, message, unknown and malformed variants are labelled
//! supplements. This crate needs only the `decoder` feature: no runtime,
//! network or credential.

mod support;

use manja::kite::decoder::framing::DecodeDiagnosticKind;
use manja::kite::decoder::text::{parse, TextEvent, TextLimits};
use manja::kite::protocol::Inbound;

use support::fixtures;

fn wrap(message_type: &str, data: &str) -> String {
    format!("{{\"type\":\"{message_type}\",\"data\":{data}}}")
}

#[test]
fn an_order_message_parses_the_official_postback_field_by_field() {
    let postback = fixtures::json_body("postback.json").unwrap();
    let TextEvent::Order(u) = parse(&wrap("order", &postback), TextLimits::default()).unwrap()
    else {
        panic!("an order update")
    };
    assert_eq!(u.order_id, "220303000308932");
    assert_eq!(u.exchange_order_id.as_deref(), Some("1000000001482421"));
    assert_eq!(u.parent_order_id, None);
    assert_eq!(u.status.as_ref().unwrap().as_wire(), "COMPLETE");
    assert_eq!(u.tradingsymbol.as_deref(), Some("SBIN"));
    assert_eq!(u.instrument_token.unwrap().get(), 779521);
    assert_eq!(u.quantity, Some(1));
    assert_eq!(u.filled_quantity, Some(1));
    assert_eq!(u.average_price, Some(470.0));
    assert_eq!(
        u.order_timestamp.unwrap().to_rfc3339(),
        "2022-03-03T09:24:25+05:30"
    );
    // The checksum is kept for the application, never verified here.
    let rendered = format!("{u:?}");
    assert!(!rendered.contains("2011845d9348bd"), "{rendered}");
}

#[test]
fn update_and_unknown_statuses_are_preserved_not_coerced() {
    // Supplemental, derived from postback.json: two status values.
    let postback = fixtures::json_body("postback.json").unwrap();
    for (status, known) in [("UPDATE", true), ("TRIGGER_WAIT_X", false)] {
        let body = postback.replace("\"COMPLETE\"", &format!("\"{status}\""));
        let TextEvent::Order(u) = parse(&wrap("order", &body), TextLimits::default()).unwrap()
        else {
            panic!()
        };
        let s = u.status.unwrap();
        assert_eq!(s.as_wire(), status);
        assert_eq!(s.known().is_some(), known, "{status}");
        assert_eq!(matches!(s, Inbound::Unknown(_)), !known);
    }
    // An unknown variety is preserved and is not usable as a known value.
    let body = postback.replace("\"regular\"", "\"basket\"");
    let TextEvent::Order(u) = parse(&wrap("order", &body), TextLimits::default()).unwrap() else {
        panic!()
    };
    assert!(u.variety.unwrap().known().is_none());
}

#[test]
fn error_message_and_unknown_types_are_distinguished() {
    let l = TextLimits::default();
    let TextEvent::Error(e) = parse(&wrap("error", "\"Invalid instrument\""), l).unwrap() else {
        panic!()
    };
    assert_eq!(e.as_str(), "Invalid instrument");
    let TextEvent::Message(m) = parse(&wrap("message", "\"Market closed\""), l).unwrap() else {
        panic!()
    };
    assert_eq!(m.as_str(), "Market closed");
    match parse(&wrap("alert_v2", "{\"level\":3}"), l).unwrap() {
        TextEvent::Unknown { message_type, data } => {
            assert_eq!(message_type.as_str(), "alert_v2");
            assert_eq!(data.unwrap()["level"], 3);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn malformed_and_oversized_text_fails_without_panic() {
    use DecodeDiagnosticKind::*;
    let l = TextLimits::default();
    let cases = [
        ("not json", InvalidText),
        ("[1,2]", InvalidText),
        ("{\"data\":1}", InvalidText),
        ("{\"type\":7}", InvalidText),
        (&*wrap("order", "\"text\"") as &str, InvalidText),
        (&*wrap("order", "{\"status\":\"COMPLETE\"}"), InvalidText),
        (&*wrap("error", "{\"code\":1}"), InvalidText),
        ("", InvalidText),
    ];
    for (text, kind) in cases {
        let e = parse(text, l).unwrap_err();
        assert_eq!(e.kind(), kind, "{text}");
        assert!(e.detail().as_str().len() <= 512);
    }
    let small = TextLimits::default().with_max_bytes(1024).unwrap();
    let big = wrap("message", &format!("\"{}\"", "x".repeat(2000)));
    assert_eq!(parse(&big, small).unwrap_err().kind(), Oversized);
    assert!(TextLimits::default().with_max_bytes(1023).is_err());
}
