//! The common protocol slice: identifiers, units, scales, broker datetimes,
//! unknown-preserving enums and the partial order-update DTO.
//!
//! Compiled in every feature build and free of async runtimes, network
//! transports and telemetry. HTTP, ticker and decoder code share these types
//! instead of raw integers and ad hoc parsing.
//!
//! - [`ids`] holds [`InstrumentToken`].
//! - [`units`] holds exact [`ScaledPrice`] values, positive [`Quantity`], and
//!   checked float conversion under [`CONVERSION_POLICY_VERSION`].
//! - [`scale`] holds the per-segment price divisor policy.
//! - [`datetime`] holds the single broker datetime parser (naive IST,
//!   UTC+05:30).
//! - [`enums`] holds [`Inbound`], which keeps unknown broker strings instead
//!   of coercing them.
//! - [`order_update`] holds [`OrderUpdate`], the partial order-update
//!   (postback) DTO shared by HTTP and WebSocket text handling.
//!
pub mod datetime;
pub mod enums;
pub mod ids;
pub mod order_update;
pub mod scale;
pub mod units;

pub use datetime::{
    BrokerTimestamp, DateTimeError, IST_OFFSET_SECONDS, parse_broker_date, parse_broker_datetime,
};
pub use enums::{Inbound, UnknownValue, WireEnum};
pub use ids::{InstrumentToken, MfOrderId, OrderId, OrderIdError};
pub use order_update::OrderUpdate;
pub use scale::{ScaleError, Segment, price_divisor};
pub use units::{CONVERSION_POLICY_VERSION, Quantity, ScaledPrice, UnitError};
