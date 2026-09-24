//! One resource per part of the Kite Connect API.
//!
//! You don't create these yourself: each is borrowed from an
//! [`HTTPClient`](crate::kite::connect::client::HTTPClient), for example
//! `client.orders()` or `client.portfolio()`.
//!
//! | Resource | Covers |
//! |---|---|
//! | [`Session`] | exchanging a request token for a session, and invalidating it |
//! | [`User`] | the profile, and funds and margins |
//! | [`Orders`] | placing, modifying and cancelling orders; the order book and trades |
//! | [`Gtt`] | Good Till Triggered orders |
//! | [`Portfolio`] | holdings, positions, auctions and holdings authorisation |
//! | [`Market`] | instruments, quotes and historical candles |
//! | [`MutualFunds`] | mutual fund orders, SIPs, holdings and the fund list, read-only |
//! | [`Margins`] and [`Charges`] | margin and charge calculations for orders you might place |
//!
//! Calls that can change your account make exactly one attempt; reads retry
//! transient failures on their own.
//!
// Manages the `/session/` API group, including authentication and session management.
mod session;
pub use session::Session;

// Manages the `/user/` API group, providing access to user-specific data
// and settings.
mod user;
pub use user::User;

// Manages the `/orders/` API group, facilitating order placement, modification,
// and status checks.
mod orders;
pub use orders::Orders;

// Manages the `/mf/` API group: mutual fund orders, SIPs, holdings and
// instruments.
mod mutual_funds;
pub use mutual_funds::MutualFunds;

// Manages the `/gtt/` API group: Good Till Triggered orders.
mod gtt;
pub use gtt::Gtt;

// Manages the `/portfolio/` API group, including holdings and positions.
//
mod portfolio;
pub use portfolio::Portfolio;

// Manages the `/instruments/` and `/quote/` API group, providing market data
// and instrument information.
mod market;
pub use market::Market;

// Manages the `/margins/` and `/charges/` API group, dealing with margin
// requirements and charges.
mod margins;
pub use margins::{Charges, Margins};
