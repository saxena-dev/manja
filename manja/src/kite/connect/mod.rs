//! Async HTTP client and additional functionality.
//!
//! This module serves as the main entry point for the `manja` crate. It includes
//! submodules that provide functionality for interacting with Kite Connect trading
//! APIs, managing configurations, handling user credentials, and working with
//! various models and utilities.
//!
pub mod api;

pub mod client;

pub mod config;

pub mod credentials;

pub mod models;

mod utils;
