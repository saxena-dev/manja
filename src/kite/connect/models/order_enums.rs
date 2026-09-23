//! Order related enums.
//!
//! Each enum lists the values documented in
//! `kite-api-docs/docs/connect/v3/orders.md:19-40` (and, for
//! [`OrderStatus::Update`], `postbacks.md:67`). `Display` and `Serialize`
//! produce the exact wire string, which is also used for URL routing.
//!
//! Deserializing one of these enums directly is strict: an unknown string is
//! an error. Response DTOs wrap them in
//! [`Inbound`](crate::kite::protocol::Inbound), which preserves an unknown
//! value instead; request DTOs take the plain enum, so an unknown inbound
//! value can never be sent as a command.
//!
use crate::kite::protocol::enums::wire_enum;

/// The variety of an order, which selects the placement route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrderVariety {
    /// Regular order.
    Regular,
    /// After Market Order.
    AfterMarket,
    /// Cover Order.
    Cover,
    /// Iceberg Order.
    Iceberg,
    /// Auction Order.
    Auction,
}

wire_enum!(OrderVariety {
    Regular => "regular",
    AfterMarket => "amo",
    Cover => "co",
    Iceberg => "iceberg",
    Auction => "auction",
});

/// The status of an order.
///
/// The most common statuses are OPEN, COMPLETE, CANCELLED and REJECTED; an
/// order passes through several interim statuses. Statuses not listed here
/// arrive as [`Inbound::Unknown`](crate::kite::protocol::Inbound::Unknown).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrderStatus {
    /// The order is open.
    Open,
    /// The order has been completely filled.
    Complete,
    /// The order has been cancelled.
    Cancelled,
    /// The order has been rejected.
    Rejected,
    /// Order request has been received by the backend.
    PutOrderReqReceived,
    /// Order pending validation by the RMS (Risk Management System).
    ValidationPending,
    /// Order is pending registration at the exchange.
    OpenPending,
    /// Order's modification values are pending validation by the RMS.
    ModifyValidationPending,
    /// Order's modification values are pending registration at the exchange.
    ModifyPending,
    /// Order's placed but the fill is pending based on a trigger price.
    TriggerPending,
    /// Order's cancellation request is pending registration at the exchange.
    CancelPending,
    /// Same as `PUT ORDER REQ RECEIVED`, but for AMOs (After Market Orders).
    AmoReqReceived,
    /// Postback-only status: an open order was modified or partially filled
    /// (`postbacks.md:3`). It says nothing about the final order state.
    Update,
}

wire_enum!(OrderStatus {
    Open => "OPEN",
    Complete => "COMPLETE",
    Cancelled => "CANCELLED",
    Rejected => "REJECTED",
    PutOrderReqReceived => "PUT ORDER REQ RECEIVED",
    ValidationPending => "VALIDATION PENDING",
    OpenPending => "OPEN PENDING",
    ModifyValidationPending => "MODIFY VALIDATION PENDING",
    ModifyPending => "MODIFY PENDING",
    TriggerPending => "TRIGGER PENDING",
    CancelPending => "CANCEL PENDING",
    AmoReqReceived => "AMO REQ RECEIVED",
    Update => "UPDATE",
});

/// The type of an order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrderType {
    /// Market order.
    Market,
    /// Limit order.
    Limit,
    /// Stoploss order.
    Stoploss,
    /// Stoploss-market order.
    StoplossMarket,
}

wire_enum!(OrderType {
    Market => "MARKET",
    Limit => "LIMIT",
    Stoploss => "SL",
    StoplossMarket => "SL-M",
});

/// The margin product of an order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProductType {
    /// Cash & Carry for equity.
    CashAndCarry,
    /// Normal for futures and options.
    Normal,
    /// Margin Intraday Squareoff for futures and options.
    MarginIntradaySquareoff,
    /// Margin Trading Facility.
    MarginTradingFacility,
}

wire_enum!(ProductType {
    CashAndCarry => "CNC",
    Normal => "NRML",
    MarginIntradaySquareoff => "MIS",
    MarginTradingFacility => "MTF",
});

/// The validity of an order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrderValidity {
    /// Regular order.
    Day,
    /// Immediate or Cancel.
    ImmediateOrCancel,
    /// Order validity in minutes.
    TimeToLive,
}

wire_enum!(OrderValidity {
    Day => "DAY",
    ImmediateOrCancel => "IOC",
    TimeToLive => "TTL",
});

/// The transaction type, `BUY` or `SELL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransactionType {
    /// Buy.
    BUY,
    /// Sell.
    SELL,
}

wire_enum!(TransactionType {
    BUY => "BUY",
    SELL => "SELL",
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kite::protocol::Inbound;

    #[test]
    fn wire_strings_round_trip_exactly() {
        assert_eq!(OrderVariety::AfterMarket.to_string(), "amo");
        assert_eq!(
            serde_json::to_string(&OrderType::StoplossMarket).unwrap(),
            "\"SL-M\""
        );
        let s: OrderStatus = serde_json::from_str("\"PUT ORDER REQ RECEIVED\"").unwrap();
        assert_eq!(s, OrderStatus::PutOrderReqReceived);
        let s: Inbound<OrderStatus> = serde_json::from_str("\"UPDATE\"").unwrap();
        assert_eq!(s.known(), Some(&OrderStatus::Update));
    }

    #[test]
    fn unknown_inbound_values_are_preserved_but_not_outbound() {
        let p: Inbound<ProductType> = serde_json::from_str("\"BO\"").unwrap();
        assert_eq!(p.as_wire(), "BO");
        assert!(ProductType::try_from(p).is_err());
        assert!(serde_json::from_str::<ProductType>("\"BO\"").is_err());
    }
}
