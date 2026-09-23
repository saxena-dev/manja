//! The HTTP client against a local server that serves the official
//! `kiteconnect-mocks` responses: account reads, an order with an explicit
//! dispatch permit, quotes, and the client's own telemetry.
//!
//! `cargo run --example http_client`. Nothing leaves `127.0.0.1`.

#[path = "support/mod.rs"]
mod support;

use std::sync::Arc;

use manja::kite::connect::client::HTTPClient;
use manja::kite::connect::config::Config;
use manja::kite::connect::credentials::Credentials;
use manja::kite::connect::models::{
    Exchange, FullQuote, OrderType, OrderVariety, PlaceOrderRequest, ProductType, TransactionType,
};
use manja::kite::connect::scheduler::PermitTarget;
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
    let client = HTTPClient::with_observability(Config::new(base), obs)?
        .with_credentials(Credentials::new("api_key", "access_token")?);

    let profile = client.user().profile().await?.data.expect("data");
    println!("profile: {} ({})", profile.user_name, profile.user_id);
    let margins = client.user().margins().await?;
    println!("margins envelope status: {}", margins.status);
    let holdings = client
        .portfolio()
        .get_holdings()
        .await?
        .data
        .unwrap_or_default();
    println!("holdings: {}", holdings.len());
    let positions = client
        .portfolio()
        .get_positions()
        .await?
        .data
        .expect("data");
    println!(
        "positions: {} net, {} day",
        positions.net.len(),
        positions.day.len()
    );
    let orders = client
        .orders()
        .list_orders()
        .await?
        .data
        .unwrap_or_default();
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
    );
    request.validate()?;
    // Admission is explicit: the permit reserves quota for this one order and
    // expires after B-HTTP-12. Placement makes exactly one attempt; a lost
    // response is reported with its stage, never retried or assumed sent.
    let permit = client.admit(PermitTarget::PlaceOrder).await?;
    let receipt = client
        .orders()
        .place_order_with_permit(&request, permit)
        .await?
        .data
        .expect("data");
    // A receipt means the broker accepted the request, not that it filled.
    println!("order accepted: {}", receipt.order_id);

    // Quotes: every requested key is accounted for.
    let quotes = client
        .market()
        .get_quotes::<FullQuote>(&["NSE:INFY", "NSE:TCS"])
        .await?
        .data
        .expect("data");
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
