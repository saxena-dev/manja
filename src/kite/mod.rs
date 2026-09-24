//! Kite module for interacting with Kite Connect API.
//!
//! Submodules are grouped by the Cargo feature that compiles them:
//!
//! - always compiled: `connect` (credentials and HTTP response models; its HTTP
//!   client and resources need `http`), `envelope`, `error`, `obs` and
//!   `protocol`;
//! - `http`: the HTTP client and resources in `connect`, and `traits`;
//! - `ticker`: `ticker`, with the legacy WebSocket client and the single-owner
//!   `actor`;
//! - `decoder`: `decoder`, pure binary and text decoding.
//!
pub mod connect;
#[cfg(feature = "decoder")]
pub mod decoder;
pub mod envelope;
pub mod error;
pub mod obs;
pub mod protocol;
#[cfg(feature = "ticker")]
pub mod ticker;
#[cfg(feature = "http")]
pub mod traits;
