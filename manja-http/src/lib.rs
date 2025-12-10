//! HTTP client and API group implementations for the `manja` SDK.
//!
//! This crate hosts the HTTP transport layer for the `manja` workspace. It
//! provides an asynchronous `HTTPClient` type built on top of `reqwest` and
//! exposes typed API groups for each Kite Connect domain (session, user,
//! orders, portfolio, market data, and margins/charges).

#![warn(rust_2018_idioms)]

pub mod api;
pub mod client;
pub mod config;
pub mod credentials;
pub mod error;
pub mod utils;

/// Test helpers for HTTP fixtures and mock servers.
///
/// This module is only compiled when running tests for this crate.
#[cfg(test)]
#[doc(hidden)]
pub mod test_utils;

pub use crate::api::{
    Alerts, BackoffPolicy, Charges, Gtt, Historical, Margins, Market, MutualFunds, Orders,
    Portfolio, Session, User,
};
pub use crate::client::HTTPClient;
pub use crate::config::Config;
pub use crate::credentials::KiteCredentials;
pub use crate::error::{Error, Result};
