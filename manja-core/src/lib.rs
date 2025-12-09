//! Core types and shared models for the `manja` SDK.
//!
//! This crate hosts transport-agnostic domain models and error descriptions
//! that are shared across the `manja` workspace. It deliberately avoids
//! depending on async runtimes or HTTP/WebSocket clients.

pub mod models;
pub mod error;
pub mod traits;
