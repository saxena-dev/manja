//! The ticker task and the types you use to control and observe it.
//!
//! - [`owner`]: [`TickerBuilder`](owner::TickerBuilder), the handle, the
//!   event stream and the task guard. Start here.
//! - [`subscriptions`]: the subscriptions you ask for, and the revisions
//!   that track each change.
//! - [`lifecycle`]: connecting, reconnecting and their limits.
//! - [`delivery`]: how events are queued and delivered to you.
//! - [`status`]: the snapshot [`TickerHandle::status`](owner::TickerHandle::status)
//!   returns.
//!
pub mod delivery;
pub mod lifecycle;
pub mod owner;
pub mod status;
pub mod subscriptions;
