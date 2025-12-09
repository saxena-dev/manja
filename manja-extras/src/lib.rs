//! Optional extras for the `manja` SDK.
//!
//! This crate hosts optional, higher-level helpers that build on top of the
//! core `manja` workspace. The primary focus in Phase 2 is to provide a
//! WebDriver/TOTP-based login flow that can be used by the facade crate
//! (`manja`) or directly by advanced consumers.
//!
//! The WebDriver runtime and TOTP helpers live here so that applications can
//! opt into these heavier dependencies explicitly.
//!
//! # Using `manja-extras` directly
//!
//! In most cases you can rely on the `manja` facade crate with the
//! `webdriver-login` feature enabled, which re-exports these helpers under
//! `manja::kite::login`. If you need more control, you can depend on this
//! crate directly:
//!
//! ```toml
//! [dependencies]
//! manja-extras = { version = "0.3" }
//! manja-core = { version = "0.3" }
//! ```
//!
//! and then drive the login flow yourself:
//!
//! ```ignore
//! use manja_extras::browser_login_flow;
//! use manja_core::traits::CoreConfig;
//!
//! async fn login_with_extras<C>(config: C) -> Result<String, manja_extras::login::LoginError>
//! where
//!     C: CoreConfig + Send,
//! {
//!     browser_login_flow(config).await
//! }
//! ```

/// WebDriver + TOTP-based login helpers.
pub mod login;

pub use crate::login::{browser_login_flow, generate_totp, launch_browser};
