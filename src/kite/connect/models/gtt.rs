//! GTT (Good Till Triggered) types (`kite:gtt.md`).
//!
//! A GTT is a trigger held by the broker: when the instrument's price
//! reaches one of the trigger values, the broker places the matching order.
//! Placing, modifying or deleting a GTT returns a [`GttReceipt`] naming the
//! trigger. That acknowledges the request only; it says nothing about
//! whether the trigger fired or an order was placed, which
//! [`GttTrigger::orders`] reports.
//!
//! Type, status, exchange and order strings in responses are [`Inbound`]
//! values, so an undocumented one is preserved rather than failing the
//! record. Timestamps are offset-free IST strings parsed into
//! `DateTime<FixedOffset>` at +05:30.
//!
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::kite::connect::models::exchange::Exchange;
use crate::kite::connect::models::order::{invalid, RequestError};
use crate::kite::connect::models::order_enums::{
    OrderType, OrderValidity, ProductType, TransactionType,
};
use crate::kite::protocol::datetime::serde_opt_datetime;
use crate::kite::protocol::enums::wire_enum;
use crate::kite::protocol::{Inbound, InstrumentToken, Quantity};

/// The kind of a GTT (`kite:gtt.md:80-167`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GttType {
    /// One trigger value and one order.
    Single,
    /// Two trigger values and two orders, one cancelling the other (OCO).
    TwoLeg,
}

wire_enum!(GttType {
    Single => "single",
    TwoLeg => "two-leg",
});

impl GttType {
    /// The number of trigger values and orders this kind takes.
    pub const fn legs(self) -> usize {
        match self {
            Self::Single => 1,
            Self::TwoLeg => 2,
        }
    }
}

/// The state of a GTT (`kite:gtt.md:359-371`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GttStatus {
    /// The trigger is active.
    Active,
    /// The trigger fired.
    Triggered,
    /// The trigger is disabled and needs action from the user.
    Disabled,
    /// The trigger reached its expiry date.
    Expired,
    /// The broker cancelled the trigger.
    Cancelled,
    /// The broker rejected the trigger.
    Rejected,
    /// The user deleted the trigger.
    Deleted,
}

wire_enum!(GttStatus {
    Active => "active",
    Triggered => "triggered",
    Disabled => "disabled",
    Expired => "expired",
    Cancelled => "cancelled",
    Rejected => "rejected",
    Deleted => "deleted",
});

/// The acknowledgement of a GTT placement, modification or deletion
/// (`kite:gtt.md:24-26,386-388,403-405`).
///
/// It names the trigger the request was registered against. It is not
/// confirmation that the trigger is active, fired or placed an order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GttReceipt {
    /// The trigger ID.
    pub trigger_id: u64,
}

/// A GTT as reported by `GET /gtt/triggers` or `GET /gtt/triggers/{id}`
/// (`kite:gtt.md:169-357`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GttTrigger {
    /// Trigger ID.
    pub id: u64,
    /// The user the trigger belongs to.
    pub user_id: String,
    /// The trigger this one was derived from, if any.
    #[serde(default)]
    pub parent_trigger: Option<u64>,
    /// Single or two-leg.
    #[serde(rename = "type")]
    pub trigger_type: Inbound<GttType>,
    /// When the trigger was created (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub created_at: Option<DateTime<FixedOffset>>,
    /// When the trigger last changed (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub updated_at: Option<DateTime<FixedOffset>>,
    /// When the trigger expires (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub expires_at: Option<DateTime<FixedOffset>>,
    /// Current state.
    pub status: Inbound<GttStatus>,
    /// What the trigger watches.
    pub condition: GttCondition,
    /// The orders the trigger places, in trigger-value order.
    pub orders: Vec<GttOrder>,
    /// Arbitrary fields the broker may attach; `{}` or `null` in the
    /// documented examples.
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
}

/// The condition of a GTT in a response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GttCondition {
    /// Exchange.
    pub exchange: Inbound<Exchange>,
    /// Exchange tradingsymbol.
    pub tradingsymbol: String,
    /// Trigger values, one per order.
    pub trigger_values: Vec<f64>,
    /// Last price when the trigger was placed or modified.
    pub last_price: f64,
    /// Instrument token.
    #[serde(default)]
    pub instrument_token: Option<InstrumentToken>,
}

/// One order of a GTT in a response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GttOrder {
    /// Exchange.
    pub exchange: Inbound<Exchange>,
    /// Exchange tradingsymbol.
    pub tradingsymbol: String,
    /// Margin product.
    pub product: Inbound<ProductType>,
    /// Order type.
    pub order_type: Inbound<OrderType>,
    /// BUY or SELL.
    pub transaction_type: Inbound<TransactionType>,
    /// Quantity.
    pub quantity: u32,
    /// Limit price.
    pub price: f64,
    /// What happened when this order's trigger fired; `None` until then.
    #[serde(default)]
    pub result: Option<GttOrderResult>,
}

/// The order a fired trigger attempted to place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GttOrderResult {
    /// The account the order was placed for.
    pub account_id: String,
    /// Exchange.
    pub exchange: Inbound<Exchange>,
    /// Exchange tradingsymbol.
    pub tradingsymbol: String,
    /// Order validity.
    pub validity: Inbound<OrderValidity>,
    /// Margin product.
    pub product: Inbound<ProductType>,
    /// Order type.
    pub order_type: Inbound<OrderType>,
    /// BUY or SELL.
    pub transaction_type: Inbound<TransactionType>,
    /// Quantity.
    pub quantity: u32,
    /// Limit price.
    pub price: f64,
    /// Broker metadata, as the JSON text the broker sent.
    #[serde(default)]
    pub meta: Option<String>,
    /// When the trigger fired (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub timestamp: Option<DateTime<FixedOffset>>,
    /// The price at which the trigger fired.
    pub triggered_at: f64,
    /// The outcome of the placement.
    pub order_result: GttOrderOutcome,
}

/// The outcome of the order a fired trigger attempted to place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GttOrderOutcome {
    /// The broker's status string, such as `failed`.
    pub status: String,
    /// The placed order's ID; `None` when the broker sent an empty one.
    #[serde(default, deserialize_with = "empty_as_none")]
    pub order_id: Option<String>,
    /// Why the placement was rejected, if it was.
    #[serde(default)]
    pub rejection_reason: Option<String>,
}

fn empty_as_none<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.filter(|s| !s.is_empty()))
}

// --- [ Request DTOs ] ---

/// A new or replacement GTT: `POST /gtt/triggers` or
/// `PUT /gtt/triggers/{id}`, form-encoded with JSON `condition` and `orders`
/// fields (`kite:gtt.md:13-78,373-393`).
///
/// Every order is for the condition's exchange and tradingsymbol, so the two
/// can never disagree. Build one with [`Self::single`] or
/// [`Self::two_leg`]; [`Self::validate`] checks the documented shape.
#[derive(Clone, Debug, PartialEq)]
pub struct GttRequest {
    /// Single or two-leg.
    pub trigger_type: GttType,
    /// Exchange. `NONE` and `INDICES` are rejected.
    pub exchange: Exchange,
    /// Exchange tradingsymbol.
    pub tradingsymbol: String,
    /// Trigger values: one for single, two for two-leg.
    pub trigger_values: Vec<f64>,
    /// The instrument's last price at the time of the request.
    pub last_price: f64,
    /// The orders to place, one per trigger value, in the same order.
    pub orders: Vec<GttOrderRequest>,
}

/// One order of a [`GttRequest`].
#[derive(Clone, Debug, PartialEq)]
pub struct GttOrderRequest {
    /// BUY or SELL.
    pub transaction_type: TransactionType,
    /// Quantity to transact.
    pub quantity: Quantity,
    /// Order type; the documentation accepts `LIMIT` only.
    pub order_type: OrderType,
    /// Margin product.
    pub product: ProductType,
    /// Limit price.
    pub price: f64,
}

impl GttOrderRequest {
    /// A LIMIT order.
    pub fn limit(
        transaction_type: TransactionType,
        quantity: Quantity,
        product: ProductType,
        price: f64,
    ) -> Self {
        Self {
            transaction_type,
            quantity,
            order_type: OrderType::Limit,
            product,
            price,
        }
    }
}

impl GttRequest {
    /// A single-leg GTT: when `trigger` is reached, place `order`.
    pub fn single(
        exchange: Exchange,
        tradingsymbol: impl Into<String>,
        trigger: f64,
        last_price: f64,
        order: GttOrderRequest,
    ) -> Self {
        Self {
            trigger_type: GttType::Single,
            exchange,
            tradingsymbol: tradingsymbol.into(),
            trigger_values: vec![trigger],
            last_price,
            orders: vec![order],
        }
    }

    /// A two-leg (OCO) GTT: when either trigger is reached, place its order.
    pub fn two_leg(
        exchange: Exchange,
        tradingsymbol: impl Into<String>,
        triggers: [f64; 2],
        last_price: f64,
        orders: [GttOrderRequest; 2],
    ) -> Self {
        Self {
            trigger_type: GttType::TwoLeg,
            exchange,
            tradingsymbol: tradingsymbol.into(),
            trigger_values: triggers.to_vec(),
            last_price,
            orders: orders.to_vec(),
        }
    }

    /// The request that would recreate `trigger`, as the starting point for a
    /// modification: the documentation recommends fetching the trigger,
    /// changing its values and sending it back (`kite:gtt.md:390-393`).
    ///
    /// `last_price` is copied from the trigger's condition, which records it
    /// as of the last placement or modification; set the current price
    /// before sending. The conversion fails, and nothing can be sent, when
    /// the trigger holds a value a request cannot carry: an unknown type,
    /// exchange, transaction type, order type or product, a zero quantity,
    /// or an order for another instrument than the condition's.
    pub fn from_trigger(trigger: &GttTrigger) -> Result<Self, RequestError> {
        fn known<T: Copy + crate::kite::protocol::WireEnum>(
            v: &Inbound<T>,
            field: &'static str,
        ) -> Result<T, RequestError> {
            match v.known() {
                Some(k) => Ok(*k),
                None => Err(RequestError {
                    field,
                    reason: "holds a value this build does not know",
                }),
            }
        }
        let condition = &trigger.condition;
        let exchange = known(&condition.exchange, "exchange")?;
        let orders = trigger
            .orders
            .iter()
            .map(|o| {
                if o.exchange != condition.exchange || o.tradingsymbol != condition.tradingsymbol {
                    return Err(RequestError {
                        field: "orders",
                        reason: "an order is for another instrument than the condition",
                    });
                }
                Ok(GttOrderRequest {
                    transaction_type: known(&o.transaction_type, "transaction_type")?,
                    quantity: Quantity::new(o.quantity).map_err(|_| RequestError {
                        field: "quantity",
                        reason: "must be positive",
                    })?,
                    order_type: known(&o.order_type, "order_type")?,
                    product: known(&o.product, "product")?,
                    price: o.price,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            trigger_type: known(&trigger.trigger_type, "type")?,
            exchange,
            tradingsymbol: condition.tradingsymbol.clone(),
            trigger_values: condition.trigger_values.clone(),
            last_price: condition.last_price,
            orders,
        })
    }

    /// Check the documented shape: a tradable exchange, a tradingsymbol,
    /// one trigger value and one order per leg (one for single, two for
    /// two-leg), finite positive trigger values and prices, a finite
    /// non-negative last price, and LIMIT orders.
    pub fn validate(&self) -> Result<(), RequestError> {
        if !self.exchange.is_tradable() {
            return invalid("exchange", "is not a tradable exchange");
        }
        if self.tradingsymbol.is_empty() || self.tradingsymbol.len() > 64 {
            return invalid("tradingsymbol", "must be 1-64 bytes");
        }
        let legs = self.trigger_type.legs();
        if self.trigger_values.len() != legs {
            return invalid("trigger_values", "must have one value per leg");
        }
        if self.orders.len() != legs {
            return invalid("orders", "must have one order per leg");
        }
        if !self
            .trigger_values
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
        {
            return invalid("trigger_values", "must be finite and positive");
        }
        // The documentation gives no lower bound for the last price, and an
        // instrument that has not traded can report 0.
        if !self.last_price.is_finite() || self.last_price < 0.0 {
            return invalid("last_price", "must be finite and not negative");
        }
        for order in &self.orders {
            if order.order_type != OrderType::Limit {
                return invalid("order_type", "GTT orders must be LIMIT");
            }
            if !order.price.is_finite() || order.price <= 0.0 {
                return invalid("price", "must be finite and positive");
            }
        }
        Ok(())
    }

    /// Form fields: `type`, then `condition` and `orders` as JSON text in
    /// the documented key order.
    #[cfg_attr(not(feature = "http"), allow(dead_code))]
    pub(crate) fn form_pairs(&self) -> Vec<(&'static str, String)> {
        #[derive(Serialize)]
        struct Condition<'a> {
            exchange: String,
            tradingsymbol: &'a str,
            trigger_values: &'a [f64],
            last_price: f64,
        }
        #[derive(Serialize)]
        struct OrderWire<'a> {
            exchange: String,
            tradingsymbol: &'a str,
            transaction_type: String,
            quantity: u32,
            order_type: String,
            product: String,
            price: f64,
        }
        let condition = Condition {
            exchange: self.exchange.to_string(),
            tradingsymbol: &self.tradingsymbol,
            trigger_values: &self.trigger_values,
            last_price: self.last_price,
        };
        let orders: Vec<OrderWire<'_>> = self
            .orders
            .iter()
            .map(|o| OrderWire {
                exchange: self.exchange.to_string(),
                tradingsymbol: &self.tradingsymbol,
                transaction_type: o.transaction_type.to_string(),
                quantity: o.quantity.get(),
                order_type: o.order_type.to_string(),
                product: o.product.to_string(),
                price: o.price,
            })
            .collect();
        // Plain structs of strings and finite numbers always serialize.
        vec![
            ("type", self.trigger_type.to_string()),
            ("condition", serde_json::to_string(&condition).unwrap()),
            ("orders", serde_json::to_string(&orders).unwrap()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(price: f64) -> GttOrderRequest {
        GttOrderRequest::limit(
            TransactionType::BUY,
            Quantity::new(1).unwrap(),
            ProductType::CashAndCarry,
            price,
        )
    }

    #[test]
    fn the_form_matches_the_documented_example() {
        // kite:gtt.md:19-21.
        let r = GttRequest::single(Exchange::NSE, "INFY", 702.0, 798.0, order(702.5));
        assert_eq!(r.validate(), Ok(()));
        assert_eq!(
            r.form_pairs(),
            vec![
                ("type", "single".to_string()),
                (
                    "condition",
                    r#"{"exchange":"NSE","tradingsymbol":"INFY","trigger_values":[702.0],"last_price":798.0}"#
                        .to_string()
                ),
                (
                    "orders",
                    r#"[{"exchange":"NSE","tradingsymbol":"INFY","transaction_type":"BUY","quantity":1,"order_type":"LIMIT","product":"CNC","price":702.5}]"#
                        .to_string()
                ),
            ]
        );
    }

    #[test]
    fn legs_must_match_the_type() {
        let mut r = GttRequest::single(Exchange::NSE, "INFY", 702.0, 798.0, order(702.5));
        r.trigger_values.push(800.0);
        assert_eq!(r.validate().unwrap_err().field, "trigger_values");
        let mut r = GttRequest::two_leg(
            Exchange::NSE,
            "INFY",
            [702.0, 798.0],
            742.0,
            [order(702.5), order(798.5)],
        );
        assert_eq!(r.validate(), Ok(()));
        r.orders.pop();
        assert_eq!(r.validate().unwrap_err().field, "orders");
    }

    #[test]
    fn statuses_and_types_round_trip_and_unknowns_are_preserved() {
        for s in GttStatus::ALL_WIRE {
            assert_eq!(Inbound::<GttStatus>::from_wire(s).as_wire(), *s);
        }
        assert_eq!(
            Inbound::<GttType>::from_wire("two-leg"),
            Inbound::Known(GttType::TwoLeg)
        );
        assert!(Inbound::<GttStatus>::from_wire("paused").is_unknown());
    }

    impl GttStatus {
        const ALL_WIRE: &'static [&'static str] = &[
            "active",
            "triggered",
            "disabled",
            "expired",
            "cancelled",
            "rejected",
            "deleted",
        ];
    }
}
