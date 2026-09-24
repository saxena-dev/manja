//! Decoding of Kite ticker messages.
//!
//! The decoder turns the ticker's messages into typed values. It works on
//! plain bytes and needs no runtime, network or storage, so it decodes live
//! messages and captured ones alike. Needs the `decoder` feature.
//!
//! - To decode messages as the ticker delivers them, wrap its stream in
//!   [`TypedEvents`](crate::kite::ticker::typed::TypedEvents) (with the
//!   `ticker` feature too).
//! - To decode stored observations with their provenance, use
//!   [`adapter::Adapter`].
//! - To parse bare bytes, split a binary message with [`framing::frame`],
//!   decode each packet with [`packets::decode`], and parse text messages
//!   with [`text::parse`].
//!
//! Prices stay the integers Kite sends; [`packets::scaled`] converts one
//! with the segment you name, because the scale depends on the segment.
//! Every bound is checked. A malformed message is an error from the parsers
//! and a diagnostic from the adapter, never a panic.
//!
pub mod adapter;
pub mod framing;
pub mod packets;
pub mod text;
