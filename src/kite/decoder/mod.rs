//! Pure decoding of Kite ticker payloads.
//!
//! Compiled with the `decoder` feature. The parser works on borrowed payload
//! bytes and needs no async runtime, network transport or storage.
//!
//! - `framing`: binary message count and length framing.
//! - `packets`: binary packet families, including market depth.
//! - `text`: text messages, including partial order updates.
//! - `adapter`: attaches observation provenance to decoded events.
//!
pub mod adapter;
pub mod framing;
pub mod packets;
pub mod text;
