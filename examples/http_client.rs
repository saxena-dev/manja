//! The HTTP client against a local server that serves the official
//! `kiteconnect-mocks` responses: account reads, an order, quotes, and the
//! client's own telemetry.
//!
//! `cargo run --example http_client`. Nothing leaves `127.0.0.1`.

#[path = "support/mod.rs"]
mod support;

use std::sync::Arc;

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::{
    Exchange, FullQuote, OrderType, OrderVariety, PlaceOrderRequest, ProductType, Tag,
    TransactionType,
};
use manja::kite::obs::{InMemoryRecorder, Instrument, Observability};
use manja::kite::protocol::Quantity;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Telemetry is host-owned: the SDK installs no subscriber or exporter.
    // This example installs a formatter for its own output only.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let recorder = Arc::new(InMemoryRecorder::new());
    let obs = Observability::with_recorder(recorder.clone());

    let base = support::http_mocks(&[
        ("GET", "/user/profile", "profile.json"),
        ("GET", "/user/margins", "margins.json"),
        ("GET", "/portfolio/holdings", "holdings.json"),
        ("GET", "/portfolio/positions", "positions.json"),
        ("GET", "/orders", "orders.json"),
        ("POST", "/orders/regular", "order_response.json"),
        ("GET", "/quote", "quote.json"),
    ])
    .await;

    // Credentials are an explicit, immutable snapshot supplied by the caller.
    // The outcome stream, registered at build time, receives the outcome of
    // every mutation, including one whose future was dropped.
    let (client, mut outcomes) = HTTPClient::builder(Config::new(base))
        .observability(obs)
        .credentials(Credentials::new("api_key", "access_token")?)
        .build_with_outcomes()?;

    let profile = client.user().profile().await?.data;
    println!("profile: {} ({})", profile.user_name, profile.user_id);
    let margins = client.user().margins().await?;
    println!(
        "margins: operation {} answered {} on attempt {}",
        margins.meta.operation_id, margins.meta.http_status, margins.meta.attempt
    );
    let holdings = client.portfolio().get_holdings().await?.data;
    println!("holdings: {}", holdings.len());
    let positions = client.portfolio().get_positions().await?.data;
    println!(
        "positions: {} net, {} day",
        positions.net.len(),
        positions.day.len()
    );
    let orders = client.orders().list_orders().await?.data;
    println!("orders in the book: {}", orders.len());

    // Orders: a validated wire request. `Quantity` refuses zero.
    let request = PlaceOrderRequest::new(
        OrderVariety::Regular,
        Exchange::NSE,
        "INFY",
        TransactionType::BUY,
        OrderType::Market,
        Quantity::new(1)?,
        ProductType::CashAndCarry,
        Tag::new("example1")?,
    );
    request.validate()?;
    // The exact bytes the placement sends, and their hash, before anything
    // leaves the process.
    let wire = request.wire()?;
    println!(
        "sending {} ({} bytes, sha256 {})",
        wire.path(),
        wire.body_len(),
        wire.sha256_hex()
    );
    // Placement is admitted now or refused with a hint, never queued, and
    // makes exactly one attempt on a task the client owns; a lost response
    // is reported with its stage, never retried or assumed sent.
    let dispatched = client.orders().place_order(&request);
    println!("operation {} dispatched", dispatched.operation_id());
    let receipt = dispatched.await?.data;
    // A receipt means the broker accepted the request, not that it filled.
    println!("order accepted: {}", receipt.order_id);
    // The same outcome, as a record on the stream: what a journal would keep.
    if let Some(Ok(outcome)) = outcomes.recv().await {
        println!(
            "outcome: operation {} {:?} status {:?} order {:?}",
            outcome.operation_id, outcome.result, outcome.http_status, outcome.receipt.order_id
        );
    }

    // Quotes: every requested key is accounted for.
    let quotes = client
        .market()
        .get_quotes::<FullQuote>(&["NSE:INFY", "NSE:TCS"])
        .await?
        .data;
    println!(
        "quotes: received {:?}, missing {:?}",
        quotes.received.keys().collect::<Vec<_>>(),
        quotes.missing
    );

    // Programmatic status without any collector.
    let d = client.diagnostics();
    println!(
        "diagnostics: {} active attempts, {} recent failures",
        d.active_attempts,
        d.last_failures.len()
    );
    println!(
        "metrics: {} operations, {} attempts",
        recorder.counter_total(Instrument::HttpOperationsTotal),
        recorder.counter_total(Instrument::HttpAttemptsTotal)
    );
    Ok(())
}
