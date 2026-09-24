//! Mutual fund types (`kite:mutual-funds.md`).
//!
//! The mutual fund APIs report orders, SIPs and holdings of funds on
//! Zerodha's Coin platform, and list the funds available there. They are
//! read-only here: the documentation states that order placement cannot be
//! done through the API (`kite:mutual-funds.md:3`), and it documents no
//! endpoint that creates, changes or cancels an order or a SIP.
//!
//! Status, variety, type and frequency strings are [`Inbound`] values: the
//! known set is what the documentation names, in its tables or its
//! examples, and anything else is preserved rather than failing the record.
//! Timestamps are offset-free IST strings parsed into
//! `DateTime<FixedOffset>` at +05:30; dates are `yyyy-mm-dd`, and an empty
//! date string is `None`.
//!
use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::{Deserialize, Deserializer, Serialize};

use crate::kite::connect::models::order_enums::TransactionType;
use crate::kite::protocol::datetime::{serde_opt_date, serde_opt_datetime};
use crate::kite::protocol::enums::wire_enum;
use crate::kite::protocol::{Inbound, MfOrderId};

/// The status of a mutual fund order. "There may be other values as well"
/// (`kite:mutual-funds.md:152`); those are preserved as unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MfOrderStatus {
    /// The order is open.
    Open,
    /// The order completed.
    Complete,
    /// The order was rejected.
    Rejected,
    /// The order was cancelled.
    Cancelled,
}

wire_enum!(MfOrderStatus {
    Open => "OPEN",
    Complete => "COMPLETE",
    Rejected => "REJECTED",
    Cancelled => "CANCELLED",
});

/// The variety of a mutual fund order: `regular` and `sip` in the attribute
/// table (`kite:mutual-funds.md:161`), and `amc_sip` in its examples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MfOrderVariety {
    /// A one-time order.
    Regular,
    /// An instalment of a SIP.
    Sip,
    /// An instalment of a SIP registered with the fund house.
    AmcSip,
}

wire_enum!(MfOrderVariety {
    Regular => "regular",
    Sip => "sip",
    AmcSip => "amc_sip",
});

/// Whether a purchase is the first in a fund or an additional one
/// (`kite:mutual-funds.md:162`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MfPurchaseType {
    /// The first purchase.
    Fresh,
    /// A further purchase.
    Additional,
}

wire_enum!(MfPurchaseType {
    Fresh => "FRESH",
    Additional => "ADDITIONAL",
});

/// The status of a SIP (`kite:mutual-funds.md:350`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SipStatus {
    /// Active.
    Active,
    /// Paused.
    Paused,
    /// Cancelled.
    Cancelled,
}

wire_enum!(SipStatus {
    Active => "ACTIVE",
    Paused => "PAUSED",
    Cancelled => "CANCELLED",
});

/// How often a SIP instalment is triggered (`kite:mutual-funds.md:352`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SipFrequency {
    /// Weekly.
    Weekly,
    /// Monthly, on [`MfSip::instalment_day`].
    Monthly,
    /// Quarterly.
    Quarterly,
}

wire_enum!(SipFrequency {
    Weekly => "weekly",
    Monthly => "monthly",
    Quarterly => "quarterly",
});

/// The dividend option of a fund: `growth` and `payout` in the tables
/// (`kite:mutual-funds.md:348,458`), and `idcw` in the SIP examples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DividendType {
    /// Growth.
    Growth,
    /// Dividend payout.
    Payout,
    /// Income distribution cum capital withdrawal.
    Idcw,
}

wire_enum!(DividendType {
    Growth => "growth",
    Payout => "payout",
    Idcw => "idcw",
});

/// The scheme type of a fund in the instrument list
/// (`kite:mutual-funds.md:459`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SchemeType {
    /// Equity.
    Equity,
    /// Debt.
    Debt,
    /// Equity-linked savings scheme.
    Elss,
}

wire_enum!(SchemeType {
    Equity => "equity",
    Debt => "debt",
    Elss => "elss",
});

/// The plan of a fund (`kite:mutual-funds.md:460`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MfPlan {
    /// Direct plan.
    Direct,
    /// Regular plan.
    Regular,
}

wire_enum!(MfPlan {
    Direct => "direct",
    Regular => "regular",
});

/// A mutual fund order, from `GET /mf/orders` or `GET /mf/orders/{order_id}`
/// (`kite:mutual-funds.md:145-169`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MfOrder {
    /// Unique order ID.
    pub order_id: MfOrderId,
    /// Exchange order ID, once the order reaches the exchange.
    #[serde(default)]
    pub exchange_order_id: Option<String>,
    /// ISIN of the fund.
    pub tradingsymbol: String,
    /// Current status.
    #[serde(default)]
    pub status: Option<Inbound<MfOrderStatus>>,
    /// Textual description of the status.
    #[serde(default)]
    pub status_message: Option<String>,
    /// Folio number, for a completed purchase.
    #[serde(default)]
    pub folio: Option<String>,
    /// Name of the fund.
    pub fund: String,
    /// When the API registered the order (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub order_timestamp: Option<DateTime<FixedOffset>>,
    /// The date the exchange registered the order; `None` if it did not.
    #[serde(default, with = "serde_opt_date")]
    pub exchange_timestamp: Option<NaiveDate>,
    /// Exchange settlement ID.
    #[serde(default)]
    pub settlement_id: Option<String>,
    /// BUY or SELL.
    pub transaction_type: Inbound<TransactionType>,
    /// Amount placed for the purchase of units.
    pub amount: f64,
    /// Order variety.
    pub variety: Inbound<MfOrderVariety>,
    /// FRESH or ADDITIONAL; `None` for a SELL order.
    #[serde(default)]
    pub purchase_type: Option<Inbound<MfPurchaseType>>,
    /// Units allotted or sold.
    pub quantity: f64,
    /// Buy or sell price, when the broker sends one.
    #[serde(default)]
    pub price: Option<f64>,
    /// Last available NAV of the fund.
    pub last_price: f64,
    /// Allotted or sold NAV.
    pub average_price: f64,
    /// The user that placed the order.
    pub placed_by: String,
    /// The date of the last available NAV.
    #[serde(default, with = "serde_opt_date")]
    pub last_price_date: Option<NaiveDate>,
    /// The tag sent with the order.
    #[serde(default)]
    pub tag: Option<String>,
}

/// A SIP, from `GET /mf/sips` (`kite:mutual-funds.md:341-360`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MfSip {
    /// Unique SIP ID.
    pub sip_id: String,
    /// ISIN of the fund.
    pub tradingsymbol: String,
    /// Name of the fund.
    pub fund: String,
    /// Dividend option.
    pub dividend_type: Inbound<DividendType>,
    /// BUY or SELL.
    pub transaction_type: Inbound<TransactionType>,
    /// ACTIVE, PAUSED or CANCELLED.
    pub status: Inbound<SipStatus>,
    /// When the API registered the SIP (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub created: Option<DateTime<FixedOffset>>,
    /// How often an instalment is triggered.
    pub frequency: Inbound<SipFrequency>,
    /// The next instalment date.
    #[serde(default, with = "serde_opt_date")]
    pub next_instalment: Option<NaiveDate>,
    /// Amount of each instalment.
    pub instalment_amount: f64,
    /// Number of instalments; `-1` for a SIP active until cancelled.
    pub instalments: i64,
    /// When the last instalment was triggered (IST).
    #[serde(default, with = "serde_opt_datetime")]
    pub last_instalment: Option<DateTime<FixedOffset>>,
    /// Instalments pending; `-1` for a SIP active until cancelled.
    pub pending_instalments: i64,
    /// Day of the month for a monthly SIP; 0 for any other frequency.
    pub instalment_day: u32,
    /// Instalments completed since the start.
    pub completed_instalments: i64,
    /// The tag sent with the SIP.
    #[serde(default)]
    pub tag: Option<String>,
    /// Registration number with the fund house, when there is one. Not in
    /// the attribute table; present in the examples.
    #[serde(default)]
    pub sip_reg_num: Option<String>,
    /// SIP kind, such as `sip` or `amc_sip`. Not in the attribute table;
    /// present in the examples.
    #[serde(default)]
    pub sip_type: Option<String>,
    /// Trigger price. Not in the attribute table; present in the examples.
    #[serde(default)]
    pub trigger_price: Option<f64>,
    /// Step-up percentages keyed by `dd-mm`. Not in the attribute table;
    /// present in the examples.
    #[serde(default)]
    pub step_up: Option<BTreeMap<String, f64>>,
}

impl MfSip {
    /// Whether the SIP runs until it is cancelled (`instalments` is `-1`).
    pub fn is_open_ended(&self) -> bool {
        self.instalments == -1
    }
}

/// A mutual fund holding, from `GET /mf/holdings`
/// (`kite:mutual-funds.md:413-424`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MfHolding {
    /// Folio number; `None` for a SELL order.
    #[serde(default)]
    pub folio: Option<String>,
    /// Name of the fund.
    pub fund: String,
    /// ISIN of the fund.
    pub tradingsymbol: String,
    /// Allotted NAV for a completed BUY; selling NAV for a completed SELL.
    pub average_price: f64,
    /// Last available NAV of the fund.
    pub last_price: f64,
    /// Net returns, based on the last available NAV.
    pub pnl: f64,
    /// The date of the last available NAV; the official example sends an
    /// empty string, which is `None`.
    #[serde(default, with = "serde_opt_date")]
    pub last_price_date: Option<NaiveDate>,
    /// Units held.
    pub quantity: f64,
    /// Units pledged. Not in the attribute table; present in the examples.
    #[serde(default)]
    pub pledged_quantity: Option<f64>,
}

/// One row of the mutual fund instrument list, `GET /mf/instruments`, a CSV
/// dump (`kite:mutual-funds.md:426-463`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct MfInstrument {
    /// ISIN of the fund.
    pub tradingsymbol: String,
    /// AMC code as per the exchange.
    pub amc: String,
    /// Fund name.
    pub name: String,
    /// Whether purchases are allowed (`1`) or not (`0`).
    #[serde(deserialize_with = "zero_or_one")]
    pub purchase_allowed: bool,
    /// Whether redemptions are allowed (`1`) or not (`0`).
    #[serde(deserialize_with = "zero_or_one")]
    pub redemption_allowed: bool,
    /// Minimum amount of the first purchase.
    pub minimum_purchase_amount: f64,
    /// A purchase amount must be a multiple of this.
    pub purchase_amount_multiplier: f64,
    /// Minimum amount of an additional purchase.
    pub minimum_additional_purchase_amount: f64,
    /// Minimum quantity of a redemption.
    pub minimum_redemption_quantity: f64,
    /// A redemption quantity must be a multiple of this.
    pub redemption_quantity_multiplier: f64,
    /// Dividend option.
    #[serde(deserialize_with = "inbound_text")]
    pub dividend_type: Inbound<DividendType>,
    /// Scheme type. The official list also carries undocumented types, such
    /// as `liquid`, which are preserved as unknown.
    #[serde(deserialize_with = "inbound_text")]
    pub scheme_type: Inbound<SchemeType>,
    /// Direct or regular plan.
    #[serde(deserialize_with = "inbound_text")]
    pub plan: Inbound<MfPlan>,
    /// Settlement type, such as `T1` or `T3`.
    pub settlement_type: String,
    /// Last available NAV. The list is a daily dump, so this is not live.
    pub last_price: f64,
    /// The date of the last available NAV.
    #[serde(default, deserialize_with = "optional_csv_date")]
    pub last_price_date: Option<NaiveDate>,
}

fn zero_or_one<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    match String::deserialize(d)?.as_str() {
        "1" => Ok(true),
        "0" => Ok(false),
        other => Err(serde::de::Error::invalid_value(
            serde::de::Unexpected::Str(other),
            &"0 or 1",
        )),
    }
}

// CSV fields are text; classify them the way JSON strings are.
fn inbound_text<'de, D, T>(d: D) -> Result<Inbound<T>, D::Error>
where
    D: Deserializer<'de>,
    T: crate::kite::protocol::WireEnum,
{
    Ok(Inbound::from_wire(&String::deserialize(d)?))
}

fn optional_csv_date<'de, D: Deserializer<'de>>(d: D) -> Result<Option<NaiveDate>, D::Error> {
    serde_opt_date::deserialize(d)
}

// The only test here parses CSV, which needs the `http` feature's `csv`.
#[cfg(all(test, feature = "http"))]
mod tests {
    use super::*;

    #[test]
    fn instrument_flags_must_be_zero_or_one() {
        let header = "tradingsymbol,amc,name,purchase_allowed,redemption_allowed,minimum_purchase_amount,purchase_amount_multiplier,minimum_additional_purchase_amount,minimum_redemption_quantity,redemption_quantity_multiplier,dividend_type,scheme_type,plan,settlement_type,last_price,last_price_date";
        let parse = |row: &str| {
            let data = format!("{header}\n{row}\n");
            csv::Reader::from_reader(data.as_bytes())
                .deserialize::<MfInstrument>()
                .next()
                .unwrap()
        };
        let ok = parse("INF1,AMC,Fund,1,0,1.0,1.0,1.0,0.001,0.001,growth,liquid,direct,T1,10.5,");
        let ok = ok.unwrap();
        assert!(ok.purchase_allowed && !ok.redemption_allowed);
        assert!(ok.scheme_type.is_unknown());
        assert_eq!(ok.last_price_date, None);
        assert!(
            parse("INF1,AMC,Fund,yes,0,1.0,1.0,1.0,0.001,0.001,growth,debt,direct,T1,10.5,")
                .is_err()
        );
    }
}
