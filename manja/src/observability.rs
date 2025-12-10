//! Observability helpers for the `manja` SDK.
//!
//! This module provides small convenience functions for wiring up a
//! `tracing` subscriber so that the HTTP client and WebSocket ticker
//! spans/events become visible in your application logs or metrics
//! pipeline.
//!
//! These helpers are **optional**; if your application already configures
//! a global `tracing_subscriber`, you do not need to use them.
//!
//! # Example
//!
//! ```no_run
//! use manja::observability;
//! use manja::ManjaClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Initialize a `tracing` subscriber using `RUST_LOG` or a default level.
//!     observability::init_tracing_from_env("info");
//!
//!     let mut client = ManjaClient::from_env();
//!     // HTTP requests and ticker connections now emit `tracing` spans/events
//!     // that can be picked up by logging or metrics backends.
//!
//!     let _profile = client.user().profile().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Integrating with metrics
//!
//! These helpers are deliberately minimal and only depend on
//! `tracing-subscriber`. To export metrics, you can extend the registry
//! with your own metrics layer:
//!
//! ```ignore
//! use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
//!
//! let filter = tracing_subscriber::EnvFilter::try_from_default_env()
//!     .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
//!
//! // `metrics_layer` could come from crates like `tracing-opentelemetry`,
//! // `metrics-tracing-context`, or any custom `Layer`.
//! let metrics_layer = /* your metrics layer here */;
//!
//! tracing_subscriber::registry()
//!     .with(filter)
//!     .with(tracing_subscriber::fmt::layer())
//!     .with(metrics_layer)
//!     .init();
//! ```

use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize a global `tracing` subscriber using `RUST_LOG` or a default
/// filter level.
///
/// This is a convenience for small applications that do not already manage
/// their own global subscriber. If a subscriber is already set, this
/// function is a no-op.
pub fn init_tracing_from_env(default_filter: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .try_init();
}

