use manja::{Exchange, FullQuote, Instrument, KiteApiResponse, ManjaClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize a simple logger for the example.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    let mut client = ManjaClient::from_env();

    // Fetch the list of tradable instruments for NSE.
    let mut instruments: Vec<Instrument> = client.market().get_instruments(Exchange::NSE).await?;

    // Select a small set of well-known symbols for which we want quotes.
    let watchlist = ["INFY", "TCS", "RELIANCE"];
    let mut selected: Vec<Instrument> = instruments
        .drain(..)
        .filter(|inst| watchlist.contains(&inst.tradingsymbol.as_str()))
        .collect();

    if selected.is_empty() {
        eprintln!(
            "No matching instruments found for {:?}. \
             Ensure your instruments list is up to date.",
            watchlist
        );
        return Ok(());
    }

    // Build the query payload expected by the quotes API.
    let query: Vec<(&str, &str)> = selected.iter_mut().map(|i| i.to_query()).collect();

    // Fetch full quotes for the selected instruments.
    let quotes: KiteApiResponse<std::collections::HashMap<String, FullQuote>> =
        client.market().get_quotes::<FullQuote>(&query).await?;

    if let Some(data) = quotes.data {
        for (symbol, quote) in data {
            println!(
                "{} => last_price = {}, volume = {:?}",
                symbol, quote.last_price, quote.volume
            );
        }
    } else {
        eprintln!("No quote data returned");
    }

    Ok(())
}

