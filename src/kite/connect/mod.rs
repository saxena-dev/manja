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
//! - [`scheduler`]: deadlines and retries.
//! - [`admission`]: local enforcement of Kite's rate limits.
//! - [`wire`]: the exact bytes a mutation sends, inspectable before the
//!   call and hashable after it.
//! - [`dispatch`]: a mutation's attempt as a task the client owns, its
//!   future, and the outcome stream nothing is lost from.
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

#[cfg(feature = "http")]
pub mod dispatch;

pub mod models;

#[cfg(feature = "http")]
pub mod scheduler;

#[cfg(feature = "http")]
mod utils;

#[cfg(feature = "http")]
pub mod wire;
