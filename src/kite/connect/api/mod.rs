//! API endpoint definitions and functions for interacting with Kite Connect API.
//!
//! This module organizes the various API groups for Kite Connect API. It includes
//! submodules for managing sessions, user data, orders, GTT orders, portfolio,
//! market data, mutual funds, and margins. Each submodule corresponds to a specific set of
//! endpoints in Kite Connect API, making it easier to interact with different
//! aspects of the trading platform.
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
