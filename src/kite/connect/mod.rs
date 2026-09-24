//! Async HTTP client and additional functionality.
//!
//! This module holds the Kite Connect HTTP slice. `credentials` and `models`
//! are always compiled because the common traits and the ticker use them. The
//! HTTP client, its configuration, the resource APIs and the `admission` and
//! `scheduler` modules are compiled with the `http` feature.
//!
#[cfg(feature = "http")]
pub mod admission;

#[cfg(feature = "http")]
pub mod api;

#[cfg(feature = "http")]
pub mod client;

#[cfg(feature = "http")]
pub mod config;

pub mod credentials;

pub mod models;

#[cfg(feature = "http")]
pub mod scheduler;

#[cfg(feature = "http")]
mod utils;
