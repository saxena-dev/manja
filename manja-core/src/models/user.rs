//! User profile related types.
//!
//! This module provides structures and functions for managing user profiles,
//! margins, and other related information in the Kite Connect API. It includes
//! detailed representations of user profiles, segment details, available and
//! utilized balances, and segment kinds.
//!
use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents the user's profile, including user ID, email, name, broker details,
/// and products enabled.
///
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct UserProfile {
    /// The unique, permanent user id registered with the broker and the exchanges
    user_id: String,
    /// User's registered role at the broker. This will be individual for all retail users
    user_type: String,
    /// User's email
    email: String,
    /// User's real name
    user_name: String,
    /// Shortened version of the user's real name
    user_shortname: String,
    /// The broker ID
    broker: String,
    /// Exchanges enabled for trading on the user's account
    exchanges: Vec<String>,
    /// Margin product types enabled for the user
    products: Vec<String>,
    /// Order types enabled for the user
    order_types: Vec<String>,
    /// Full URL to the user's avatar (PNG image) if there's one
    avatar_url: Option<String>,
    /// Additional metadata
    meta: Meta,
}

/// Represents additional metadata associated with the user's profile.
///
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    /// Demat consent: empty, consent or physical
    demat_consent: String,
}

/// Represents the user's margins for equity and commodity segments.
///
/// This struct contains details about the user's funds, cash, and margin
/// information for different segments.
///
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct UserMargins {
    /// Equity segment details
    pub equity: Option<Segment>,
    /// Commodity segment details
    pub commodity: Option<Segment>,
}

/// Represents the details of a specific segment, including available and
/// utilized balances.
///
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    /// Indicates whether the segment is enabled for the user
    pub enabled: bool,
    /// Net cash balance available for trading
    /// (`intraday_payin` + `adhoc_margin` + `collateral`)
    pub net: f64,
    /// Available balance details
    pub available: Available,
    /// Utilized balance details
    pub utilised: Utilised,
}

/// Represents the available balance details within a segment.
///
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Available {
    /// Raw cash balance in the account available for trading (also includes
    /// `intraday_payin`)
    pub cash: f64,
    /// Opening balance at the day start
    pub opening_balance: f64,
    /// Current available balance
    pub live_balance: f64,
    /// Amount that was deposited during the day
    pub intraday_payin: f64,
    /// Additional margin provided by the broker
    pub adhoc_margin: f64,
    /// Margin derived from pledged stocks
    pub collateral: f64,
}

/// Represents the utilized balance details within a segment.
///
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Utilised {
    /// Sum of all utilised margins
    /// (unrealised M2M + realised M2M + SPAN + Exposure + Premium + Holding sales)
    pub debits: f64,
    /// Exposure margin blocked for all open F&O positions
    pub exposure: f64,
    /// Booked intraday profits and losses
    pub m2m_realised: f64,
    /// Un-booked (open) intraday profits and losses
    pub m2m_unrealised: f64,
    /// Value of options premium received by shorting
    pub option_premium: f64,
    /// Funds paid out or withdrawn to bank account during the day
    pub payout: f64,
    /// SPAN margin blocked for all open F&O positions
    pub span: f64,
    /// Value of holdings sold during the day
    pub holding_sales: f64,
    /// Utilised portion of the maximum turnover limit (only applicable to certain clients)
    pub turnover: f64,
    /// Margin utilised against pledged liquidbees ETFs and liquid mutual funds
    pub liquid_collateral: f64,
    /// Margin utilised against pledged stocks/ETFs
    pub stock_collateral: f64,
    /// Margin blocked when you sell securities (20% of the value of stocks sold)
    /// from your demat or T1 holdings
    pub delivery: f64,
}

/// Enum representing the different types of segments (commodity and equity).
///
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum SegmentKind {
    Commodity,
    Equity,
}

impl AsRef<str> for SegmentKind {
    fn as_ref(&self) -> &str {
        match self {
            Self::Commodity => "commodity",
            Self::Equity => "equity",
        }
    }
}

impl fmt::Display for SegmentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Commodity => "commodity",
            Self::Equity => "equity",
        };
        write!(f, "{}", s)
    }
}
