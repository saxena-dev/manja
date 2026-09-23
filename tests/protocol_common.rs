//! The common protocol slice used as a downstream consumer would, in every
//! feature build including `--no-default-features`.
//!
//! Baseline: the official `kiteconnect-mocks/postback.json`, served unchanged.
//! Expected values are written out independently from that file's contents.

#[allow(dead_code)]
#[path = "support/fixtures.rs"]
mod fixtures;

use manja::kite::connect::models::{
    Exchange, OrderStatus, OrderType, OrderValidity, OrderVariety, ProductType, TransactionType,
};
use manja::kite::protocol::{Inbound, InstrumentToken, OrderUpdate};

#[test]
fn official_postback_decodes_with_independent_field_assertions() {
    let text = fixtures::read("postback.json").unwrap();
    let u: OrderUpdate = serde_json::from_str(&text).unwrap();
    assert_eq!(u.order_id, "220303000308932");
    assert_eq!(u.exchange_order_id.as_deref(), Some("1000000001482421"));
    assert_eq!(u.parent_order_id, None);
    assert_eq!(u.status, Some(Inbound::Known(OrderStatus::Complete)));
    assert_eq!(u.status_message, None);
    assert_eq!(u.user_id.as_deref(), Some("AB1234"));
    assert_eq!(u.placed_by.as_deref(), Some("AB1234"));
    assert_eq!(u.app_id, Some(1234));
    assert_eq!(u.tradingsymbol.as_deref(), Some("SBIN"));
    assert_eq!(u.instrument_token, Some(InstrumentToken::new(779521)));
    assert_eq!(u.exchange, Some(Inbound::Known(Exchange::NSE)));
    assert_eq!(u.variety, Some(Inbound::Known(OrderVariety::Regular)));
    assert_eq!(u.order_type, Some(Inbound::Known(OrderType::Market)));
    assert_eq!(
        u.transaction_type,
        Some(Inbound::Known(TransactionType::BUY))
    );
    assert_eq!(u.validity, Some(Inbound::Known(OrderValidity::Day)));
    assert_eq!(u.product, Some(Inbound::Known(ProductType::CashAndCarry)));
    assert_eq!(u.quantity, Some(1));
    assert_eq!(u.filled_quantity, Some(1));
    assert_eq!(u.unfilled_quantity, Some(0));
    assert_eq!(u.pending_quantity, Some(0));
    assert_eq!(u.cancelled_quantity, Some(0));
    assert_eq!(u.price, Some(0.0));
    assert_eq!(u.average_price, Some(470.0));
    assert_eq!(u.market_protection, Some(0.0));
    assert_eq!(u.tag, None);
    assert_eq!(u.guid.as_deref(), Some("XXXXXX"));
    let ts = u.order_timestamp.unwrap();
    assert_eq!(ts.to_rfc3339(), "2022-03-03T09:24:25+05:30");
    assert_eq!(
        u.exchange_timestamp.unwrap().to_rfc3339(),
        "2022-03-03T09:24:25+05:30"
    );
    assert_eq!(
        u.checksum.unwrap().expose(),
        "2011845d9348bd6795151bf4258102a03431e3bb12a79c0df73fcb4b7fde4b5d"
    );
}

#[test]
fn update_postback_without_order_list_fields_decodes() {
    // Supplemental, derived from postback.json: status changed to UPDATE and
    // every field except order_id, status and the quantities removed.
    let text = r#"{"order_id": "220303000308932", "status": "UPDATE",
                   "filled_quantity": 1, "pending_quantity": 4}"#;
    let u: OrderUpdate = serde_json::from_str(text).unwrap();
    assert_eq!(u.status, Some(Inbound::Known(OrderStatus::Update)));
    assert_eq!(u.pending_quantity, Some(4));
    assert!(u.tradingsymbol.is_none() && u.order_timestamp.is_none());
}

#[test]
fn unknown_values_are_kept_and_refused_as_commands() {
    // Supplemental, derived from postback.json: status and product replaced
    // with values this build does not know.
    let text = fixtures::read("postback.json")
        .unwrap()
        .replace("\"COMPLETE\"", "\"PARTIALLY SETTLED\"")
        .replace("\"CNC\"", "\"XYZ\"");
    let u: OrderUpdate = serde_json::from_str(&text).unwrap();
    let status = u.status.unwrap();
    assert_eq!(status.as_wire(), "PARTIALLY SETTLED");
    assert!(OrderStatus::try_from(status).is_err());
    let product = u.product.unwrap();
    assert!(product.is_unknown());
    assert!(ProductType::try_from(product).is_err());
}
