//! Single-owner ticker actor.
//!
//! Compiled with the `ticker` feature, as part of the ticker module.
//!
//! - `owner`: the socket-owning task, its handle and task guard.
//! - `subscriptions`: desired subscriptions and command revisions.
//! - `lifecycle`: connection, reconnect and credential-rejection states.
//! - `delivery`: bounded delivery, cancellation and teardown.
//! - `status`: the queryable status snapshot.
//!
pub mod delivery;
pub mod lifecycle;
pub mod owner;
pub mod status;
pub mod subscriptions;
