//! Market quotes and instruments API group: `/quote/` and `/instruments/`
//!
//! This module provides functionality to fetch market quotes and the instrument
//! master CSV dump from the Kite Connect HTTP API.

use std::collections::HashMap;

use crate::api::{create_backoff_policy, BackoffPolicy};
use crate::client::HTTPClient;
use crate::error::{Error, Result};
use manja_core::models::{Exchange, Instrument, KiteApiResponse, KiteQuote, QuoteMode};

/// Market quotes APIs and instrument master endpoints.
pub struct Market<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
    /// Backoff policy for retrying API requests.
    backoff: BackoffPolicy,
}

impl<'c> Market<'c> {
    /// Creates a new instance of `Market` with default API rate limits.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self {
            client,
            // Default API rate limit: 10 req/sec
            backoff: create_backoff_policy(10),
        }
    }

    /// Sets a custom backoff policy for the `Market` instance.
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    // Parses the CSV response into a vector of `Instrument`.
    //
    // This function will return an error if the CSV data cannot be parsed.
    fn parse_instruments(data: &str) -> Result<Vec<Instrument>> {
        let mut rdr = csv::Reader::from_reader(data.as_bytes());
        let mut records = Vec::new();

        for result in rdr.deserialize() {
            let record: Instrument =
                result.map_err(|_| Error::Internal("CSV parse error".to_string()))?;
            records.push(record);
        }

        Ok(records)
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Retrieve the CSV dump of all tradable instruments on an exchange.
    ///
    /// The instrument list API returns a gzipped CSV dump of instruments across
    /// all exchanges (if not specified) that can be imported into a database.
    /// The dump is generated once everyday and hence `last_price` is not real time.
    pub async fn get_instruments_csv(&self, exchange: Option<Exchange>) -> Result<String> {
        let path = exchange.map_or("/instruments".to_string(), |x| {
            format!("/instruments/{}", x)
        });
        self.client.get_raw(&path, &self.backoff).await
    }

    /// Retrieve all tradable instruments.
    ///
    /// WARNING: The instrument list API returns large amounts of data. It's
    /// best to request it once a day and cache the instrument data.
    pub async fn get_instruments_all(&self) -> Result<Vec<Instrument>> {
        let instruments = self.get_instruments_csv(None).await?;
        Market::parse_instruments(&instruments)
    }

    /// Retrieve all tradable instruments from a particular exchange.
    pub async fn get_instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>> {
        let instruments = self.get_instruments_csv(Some(exchange)).await?;
        Market::parse_instruments(&instruments)
    }

    /// Retrieve market quotes for one or more instruments.
    ///
    /// The query vector is typically built using [`Instrument::to_query`].
    #[allow(private_bounds)]
    pub async fn get_quotes<Q>(
        &self,
        query: &Vec<(&str, &str)>,
    ) -> Result<KiteApiResponse<HashMap<String, Q>>>
    where
        Q: KiteQuote,
    {
        let (path, limit) = match Q::mode() {
            QuoteMode::Full => ("/quote", std::cmp::min(500, query.len())),
            QuoteMode::OHLC => ("/quote/ohlc", std::cmp::min(1000, query.len())),
            QuoteMode::LTP => ("/quote/ltp", std::cmp::min(1000, query.len())),
        };
        self.client
            .get_with_query(path, &query[..limit], &self.backoff)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::join;

    use crate::client::HTTPClient;
    use crate::config::{Config, KITECONNECT_API_LOGIN, KITECONNECT_API_REDIRECT};
    use crate::credentials::KiteCredentials;
    use crate::error::Error;
    use crate::test_utils::{
        add_mocks, get_http_test_client, resolve_fixture_path, APIEndpoint, HTTPMethod, TestResponse,
    };
    use manja_core::error::KiteApiException;
    use manja_core::models::{
        Exchange, FullQuote, Instrument, KiteApiResponse, LTPQuote, OHLCQuote,
    };

    use super::*;

    fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
        let mut mmap = HashMap::new();
        mmap.insert(
            ("GET", "/instruments"),
            "./kiteconnect-mocks/instruments_all.csv",
        );
        mmap.insert(
            ("GET", "/instruments/NSE"),
            "./kiteconnect-mocks/instruments_nse.csv",
        );
        mmap
    }

    #[tokio::test]
    async fn test_get_instruments_all_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let instruments = client
            .market()
            .get_instruments_all()
            .await
            .unwrap();

        assert!(!instruments.is_empty());
        let first: &Instrument = &instruments[0];
        assert_eq!(first.tradingsymbol, "CENTRALBK-BE");
    }

    #[tokio::test]
    async fn test_get_instruments_for_exchange_success() {
        let (server, mut client) = get_http_test_client().await;
        let (_server,) = join!(add_mocks(server, mock_map()));

        let instruments = client
            .market()
            .get_instruments(Exchange::NSE)
            .await
            .unwrap();

        assert!(!instruments.is_empty());
        for instrument in instruments {
            assert_eq!(instrument.exchange, Exchange::NSE);
        }
    }

    #[test]
    fn test_get_full_quotes_success() {
        let path = resolve_fixture_path("kiteconnect-mocks/quote.json");
        let json = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("failed to read fixture at {}: {}", path.display(), err)
        });
        let response: KiteApiResponse<HashMap<String, FullQuote>> =
            serde_json::from_str(&json).unwrap();
        let data = response.data.expect("expected quote data");
        let quote = data.get("NSE:INFY").expect("expected NSE:INFY quote");
        assert_eq!(quote.instrument_token, 408065);
    }

    #[test]
    fn test_get_ohlc_quotes_success() {
        let path = resolve_fixture_path("kiteconnect-mocks/ohlc.json");
        let json = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("failed to read fixture at {}: {}", path.display(), err)
        });
        let response: KiteApiResponse<HashMap<String, OHLCQuote>> =
            serde_json::from_str(&json).unwrap();
        let data = response.data.expect("expected quote data");
        let quote = data.get("NSE:INFY").expect("expected NSE:INFY quote");
        assert_eq!(quote.instrument_token, 408065);
    }

    #[test]
    fn test_get_ltp_quotes_success() {
        let path = resolve_fixture_path("kiteconnect-mocks/ltp.json");
        let json = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("failed to read fixture at {}: {}", path.display(), err)
        });
        let response: KiteApiResponse<HashMap<String, LTPQuote>> =
            serde_json::from_str(&json).unwrap();
        let data = response.data.expect("expected quote data");
        let quote = data.get("NSE:INFY").expect("expected NSE:INFY quote");
        assert_eq!(quote.instrument_token, 408065);
    }

    #[tokio::test]
    async fn test_get_instruments_all_token_exception_error() {
        let mut server = mockito::Server::new_async().await;
        let credentials = KiteCredentials::new(
            "TEST_API_KEY",
            "TEST_API_SECRET",
            "TEST_USER_ID",
            "TEST_PASSWORD",
            "TEST_TOTP",
        );
        let config = Config::from_parts(
            server.url(),
            KITECONNECT_API_LOGIN.to_string(),
            KITECONNECT_API_REDIRECT.to_string(),
            credentials,
        );
        let mut client = HTTPClient::with_config(config);

        let _m = server
            .mock("GET", "/instruments")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "status": "error",
                    "data": null,
                    "message": "Token is invalid or has expired",
                    "error_type": "TokenException"
                }"#,
            )
            .create_async()
            .await;

        let err = client
            .market()
            .get_instruments_all()
            .await
            .expect_err("expected token exception error");

        match err {
            Error::KiteApi(api_err) => {
                assert_eq!(api_err.status_code, 403);
                assert!(matches!(
                    api_err.error_type,
                    KiteApiException::TokenException
                ));
                assert_eq!(api_err.error_type.as_str(), "TokenException");
            }
            other => panic!("unexpected error variant: {:?}", other),
        }
    }
}
