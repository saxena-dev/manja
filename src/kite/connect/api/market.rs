//! Market quotes and instruments API group: `/quote/` and `/instruments/`
//! (`kite-api-docs/docs/connect/v3/market-quotes.md`).
//!
//! # Quote requests
//!
//! Each quote endpoint has a documented instrument limit: 500 for `/quote`,
//! 1000 for `/quote/ohlc` and `/quote/ltp` (`market-quotes.md:272-278`). The
//! policy for more instruments is **explicit rejection**: a request over the
//! limit is a `Validation` error before admission and nothing is sent. The
//! SDK never truncates a request and never splits it into hidden batches;
//! at the documented quote rate of one request per second
//! (`exceptions.md:49`) a batch would be several separate snapshots and
//! seconds of waiting. Callers that need more instruments issue several
//! requests and combine them knowingly.
//!
//! An empty request, a duplicate instrument key or a malformed key
//! (`EXCHANGE:TRADINGSYMBOL`) is rejected the same way. Each key is sent as a
//! separate `i` query parameter, preserving the request's multiplicity.
//! The result reports requested, received, missing and unexpected keys; a
//! missing key never becomes a zero quote.
//!
//! # Instruments
//!
//! The instrument dump is CSV (`market-quotes.md:15-48`). It is read under
//! the CSV body bound (`B-HTTP-08`, 64 MiB by default) and parsed row by row;
//! a malformed row is a `Decode` error naming its row number. Its
//! `last_price` is not a live quote.
//!
use std::collections::{HashMap, HashSet};

use crate::kite::connect::{
    client::HTTPClient,
    models::{Exchange, Instrument, KiteApiResponse, KiteQuote, Quotes},
};
use crate::kite::error::{HttpError, HttpErrorKind, Result, TransportStage};
use crate::kite::obs::schema::{Endpoint, Method};

/// Market quotes and the instrument master.
pub struct Market<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
}

fn endpoint_of(path: &str) -> Endpoint {
    match path {
        "/quote" => Endpoint::Quote,
        "/quote/ohlc" => Endpoint::QuoteOhlc,
        "/quote/ltp" => Endpoint::QuoteLtp,
        "/instruments" => Endpoint::Instruments,
        _ => Endpoint::InstrumentsExchange,
    }
}

fn rejected(path: &str, detail: &str) -> HttpError {
    HttpError::new(
        HttpErrorKind::Validation,
        Method::Get,
        endpoint_of(path),
        TransportStage::NotStarted,
    )
    .with_detail(detail)
}

/// The distinct keys of a valid quote request, or why it is invalid.
fn check_keys<'k>(
    instruments: &[&'k str],
    max: usize,
) -> std::result::Result<HashSet<&'k str>, String> {
    if instruments.is_empty() {
        return Err("at least one instrument is required".into());
    }
    if instruments.len() > max {
        return Err(format!(
            "{} instruments exceed the documented limit of {max}",
            instruments.len()
        ));
    }
    let mut seen = HashSet::with_capacity(instruments.len());
    for key in instruments {
        let well_formed = key.len() <= 128
            && key
                .split_once(':')
                .is_some_and(|(e, s)| !e.is_empty() && !s.is_empty());
        if !well_formed {
            return Err("an instrument key is not EXCHANGE:TRADINGSYMBOL".into());
        }
        if !seen.insert(*key) {
            return Err("an instrument key is repeated".into());
        }
    }
    Ok(seen)
}

impl<'c> Market<'c> {
    /// Market APIs on `client`.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self { client }
    }

    /// Parse the instrument CSV, row by row.
    fn parse_instruments(
        path: &str,
        data: &str,
    ) -> std::result::Result<Vec<Instrument>, HttpError> {
        let mut rdr = csv::Reader::from_reader(data.as_bytes());
        let mut records = Vec::new();
        for (row, result) in rdr.deserialize().enumerate() {
            let record: Instrument = result.map_err(|_| {
                HttpError::new(
                    HttpErrorKind::Decode,
                    Method::Get,
                    endpoint_of(path),
                    TransportStage::ResponseReceived,
                )
                .with_status(200)
                .with_detail(&format!("instrument CSV row {} is malformed", row + 1))
            })?;
            records.push(record);
        }
        Ok(records)
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// The raw instrument CSV of every exchange, or of `exchange`.
    ///
    /// The dump is generated once a day, so its `last_price` is not live.
    pub async fn get_instruments_csv(&self, exchange: Option<Exchange>) -> Result<String> {
        let path = match exchange {
            Some(x) => format!("/instruments/{x}"),
            None => "/instruments".to_string(),
        };
        self.client.get_raw(&path).await
    }

    /// Every tradable instrument, parsed.
    ///
    /// The dump is large; request it once a day and store it
    /// (`market-quotes.md:50-52`).
    pub async fn get_instruments_all(&self) -> Result<Vec<Instrument>> {
        self.client
            .get_csv("/instruments", |csv| {
                Market::parse_instruments("/instruments", csv)
            })
            .await
    }

    /// The instruments of one exchange, parsed.
    pub async fn get_instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>> {
        self.client
            .get_csv(&format!("/instruments/{exchange}"), |csv| {
                Market::parse_instruments("/instruments/{exchange}", csv)
            })
            .await
    }

    /// Quotes of type `Q` for instrument keys such as `NSE:INFY`.
    ///
    /// ```no_run
    /// # async fn f(client: &manja::kite::connect::client::HTTPClient) -> manja::kite::error::Result<()> {
    /// use manja::kite::connect::api::Market;
    /// use manja::kite::connect::models::LTPQuote;
    ///
    /// let quotes = Market::new(client)
    ///     .get_quotes::<LTPQuote>(&["NSE:INFY", "BSE:SENSEX"])
    ///     .await?
    ///     .data
    ///     .expect("a success envelope carries data");
    /// for key in &quotes.missing {
    ///     println!("no data for {key}");
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// Fails before admission, sending nothing, for an empty list, more keys
    /// than the endpoint's limit, a duplicate key or a malformed key.
    pub async fn get_quotes<Q>(&self, instruments: &[&str]) -> Result<KiteApiResponse<Quotes<Q>>>
    where
        Q: KiteQuote,
    {
        let mode = Q::mode();
        let path = mode.path();
        let seen = check_keys(instruments, mode.max_instruments())
            .map_err(|detail| self.client.reject(rejected(path, &detail)))?;
        let query: Vec<(&str, &str)> = instruments.iter().map(|k| ("i", *k)).collect();
        let response = self
            .client
            .get_with_query::<_, HashMap<String, Q>>(path, &query)
            .await?;
        let received = response.data.unwrap_or_default();
        let requested: Vec<String> = instruments.iter().map(|k| k.to_string()).collect();
        let missing = requested
            .iter()
            .filter(|k| !received.contains_key(*k))
            .cloned()
            .collect();
        let mut unexpected: Vec<String> = received
            .keys()
            .filter(|k| !seen.contains(k.as_str()))
            .cloned()
            .collect();
        unexpected.sort();
        Ok(KiteApiResponse {
            status: response.status,
            data: Some(Quotes {
                requested,
                received,
                missing,
                unexpected,
            }),
            message: response.message,
            error_type: response.error_type,
        })
    }
}
