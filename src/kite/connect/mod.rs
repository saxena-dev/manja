//! The Kite Connect HTTP API.
//!
//! Start with [`HTTPClient`](client::HTTPClient): create one, give it your
//! [`Credentials`](credentials::Credentials), and reach each part of the API
//! through it. The pieces:
//!
//! - [`client`]: the client itself.
//! - [`api`]: one resource per part of the API, such as orders or the
//!   portfolio.
//! - [`models`]: the requests you send and the responses you get back.
//! - [`credentials`]: API keys, access tokens and secrets, validated and
//!   kept out of logs.
//! - [`config`]: the API base URL and the runtime bounds.
//! - [`scheduler`]: deadlines, retries and dispatch permits.
//! - [`admission`]: local enforcement of Kite's rate limits.
//!
//! `credentials` and `models` are always available, because the ticker and
//! the decoder use them too. Everything else needs the `http` feature.
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
