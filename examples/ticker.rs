//! The supervised ticker against a local WebSocket server that sends
//! vendored ticker bytes: raw delivery, subscription commands, status and
//! shutdown.
//!
//! `cargo run --example ticker`. Nothing leaves `127.0.0.1`.

#[path = "support/mod.rs"]
mod support;

use futures_util::StreamExt;
use manja::kite::connect::credentials::Credentials;
use manja::kite::envelope::PayloadKind;
use manja::kite::obs::Observability;
use manja::kite::protocol::InstrumentToken;
use manja::kite::ticker::Mode;
use manja::kite::ticker::actor::owner::{TickerBuilder, TickerEvent};
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = support::ticker_mock(vec![
        Message::Binary(support::repo_file(
            "tests/fixtures/ticker/protocol/heartbeat.bin",
        )),
        Message::Binary(support::repo_file(
            "tests/fixtures/ticker/protocol/single_full.bin",
        )),
        Message::Text(r#"{"type":"message","data":"hello"}"#.into()),
    ])
    .await;

    // No collector: status and events work without any telemetry consumer.
    let (handle, mut events, guard) =
        TickerBuilder::new(Credentials::new("api_key", "access_token")?)
            .url(url)
            .observability(Observability::disabled())
            .spawn()?;

    // Commands complete when the owner accepts them, with a revision. That
    // is not broker acknowledgement, and Active is not quote freshness.
    let infy = InstrumentToken::new(408065);
    let r1 = handle.subscribe([infy], Mode::Quote).await?;
    let r2 = handle.set_mode([infy], Mode::Full).await?;
    let r3 = handle
        .subscribe([InstrumentToken::new(884737)], Mode::LTP)
        .await?;
    let r4 = handle.unsubscribe([InstrumentToken::new(884737)]).await?;
    println!("revisions: {r1:?} {r2:?} {r3:?} {r4:?}");

    let mut raw = 0;
    while raw < 3 {
        match events.next().await {
            Some(Ok(TickerEvent::Raw(o))) => {
                raw += 1;
                let what = match o.kind() {
                    PayloadKind::Binary if o.payload().len() == 1 => "heartbeat".to_string(),
                    PayloadKind::Binary => format!("{} binary bytes", o.payload().len()),
                    PayloadKind::Text => format!("text {:?}", o.text()),
                };
                println!(
                    "raw #{} (epoch {}): {what}",
                    o.source().ingress_sequence(),
                    o.source().connection_epoch().0
                );
            }
            Some(Ok(TickerEvent::Lifecycle(l))) => println!("lifecycle: {:?}", l.kind()),
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(e.into()),
            None => break,
        }
    }

    let status = handle.status();
    println!(
        "status: {:?}, desired {:?}, sent {:?}, last message {:?} ago",
        status.state, status.desired_revision, status.sent_revision, status.last_message_age
    );

    // A clean shutdown drains the remaining events, then the stream ends.
    handle.shutdown().await?;
    while let Some(item) = events.next().await {
        if let Ok(TickerEvent::Lifecycle(l)) = item {
            println!("lifecycle: {:?}", l.kind());
        }
    }
    println!("outcome: {:?}", guard.join().await);
    Ok(())
}
