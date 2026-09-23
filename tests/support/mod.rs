//! Hermetic test support: fixture loading and loopback HTTP/WebSocket
//! harnesses with fault injection. An integration test uses it with
//! `mod support;`. Nothing here reaches the network: every harness binds
//! `127.0.0.1`, and fixtures resolve from this crate's manifest directory.

// Shared support code: no single test crate uses every helper.
#![allow(dead_code)]

/// Test-only reader for the vendored capture container.
pub mod capture;
/// Fixture loading from `kiteconnect-mocks/`, independent of the working directory.
pub mod fixtures;
pub mod http;
pub mod ws;
