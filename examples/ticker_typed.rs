//! Typed ticker events: the ticker composed with the decoder. Each
//! observation arrives with its decoding; the raw bytes stay available.
//!
//! `cargo run --example ticker_typed`. Nothing leaves `127.0.0.1`.

#[path = "support/mod.rs"]
mod support;

use futures_util::StreamExt;
use manja::kite::connect::credentials::Credentials;
use manja::kite::decoder::adapter::{Adapter, DecodedEvent};
use manja::kite::decoder::packets::{Packet, scaled};
use manja::kite::obs::Observability;
use manja::kite::obs::schema::SourceMode;
use manja::kite::protocol::scale::Segment;
use manja::kite::ticker::actor::owner::TickerBuilder;
use manja::kite::ticker::typed::{TypedEvent, TypedEvents};
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = support::ticker_mock(vec![Message::Binary(support::repo_file(
        "tests/fixtures/ticker/protocol/multi_packet.bin",
    ))])
    .await;
    let obs = Observability::disabled();
    let (handle, events, _guard) = TickerBuilder::new(Credentials::new("api_key", "access_token")?)
        .url(url)
        .spawn()?;
    let mut typed = TypedEvents::new(events, Adapter::new(SourceMode::Live, &obs));
    while let Some(item) = typed.next().await {
        let TypedEvent::Observation { raw, decoded } = item? else {
            continue;
        };
        let decoded = decoded?;
        println!("{} bytes, result {:?}", raw.payload().len(), decoded.result);
        for event in decoded.events {
            if let DecodedEvent::Packet {
                packet,
                packet_index,
                ..
            } = event
            {
                // Prices are raw integers; the caller supplies the segment.
                let (token, last) = match packet {
                    Packet::Full(p) => (p.fields.instrument_token, p.fields.last_price),
                    Packet::IndexFull(p) => (p.fields.instrument_token, p.fields.last_price),
                    other => (other.instrument_token(), 0),
                };
                println!(
                    "  packet {packet_index}: token {} last {} (raw {last})",
                    token.get(),
                    scaled(last, Segment::Nse)?.to_f64()
                );
            }
        }
        break;
    }
    handle.shutdown().await?;
    Ok(())
}
