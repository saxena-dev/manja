//! Mutual funds API group: `/mf/` (`kite:mutual-funds.md`).
//!
//! Orders, SIPs and holdings of funds on Zerodha's Coin platform, and the
//! list of funds available there. Every operation is a read, retried like
//! every read and admitted in the standard quota class. The documentation
//! states that order placement cannot be done through the API
//! (`kite:mutual-funds.md:3`) and documents no endpoint that changes an
//! order or a SIP, so none is offered here.
//!
//! The instrument list is CSV (`kite:mutual-funds.md:426-463`). It is read
//! under the CSV body bound (`B-HTTP-08`) and parsed row by row; a malformed
//! row is a `Decode` error naming its row number. Its `last_price` is not a
//! live NAV.
//!
use crate::kite::connect::{
    client::HTTPClient,
    models::{KiteApiResponse, MfHolding, MfInstrument, MfOrder, MfSip},
};
use crate::kite::error::{HttpError, HttpErrorKind, Result, TransportStage};
use crate::kite::obs::schema::{Endpoint, Method};
use crate::kite::protocol::MfOrderId;

/// Mutual fund orders, SIPs, holdings and instruments.
pub struct MutualFunds<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
}

impl<'c> MutualFunds<'c> {
    /// Mutual fund APIs on `client`.
    pub fn new(client: &'c HTTPClient) -> Self {
        Self { client }
    }

    /// Orders placed in the last 7 days, open and executed: `GET /mf/orders`
    /// (`kite:mutual-funds.md:17-19`).
    pub async fn list_orders(&self) -> Result<KiteApiResponse<Vec<MfOrder>>> {
        self.client.get("/mf/orders").await
    }

    /// One order, whatever its age: `GET /mf/orders/{order_id}`
    /// (`kite:mutual-funds.md:171-173`).
    ///
    /// The ID is an [`MfOrderId`], valid by construction, so it cannot change
    /// the request path; an equity `OrderId` is a different type.
    pub async fn get_order(&self, order_id: &MfOrderId) -> Result<KiteApiResponse<MfOrder>> {
        self.client.get(&format!("/mf/orders/{order_id}")).await
    }

    /// Active and paused SIPs: `GET /mf/sips` (`kite:mutual-funds.md:209-211`).
    pub async fn list_sips(&self) -> Result<KiteApiResponse<Vec<MfSip>>> {
        self.client.get("/mf/sips").await
    }

    /// Allotted units held in the DEMAT account: `GET /mf/holdings`
    /// (`kite:mutual-funds.md:362-364`).
    pub async fn list_holdings(&self) -> Result<KiteApiResponse<Vec<MfHolding>>> {
        self.client.get("/mf/holdings").await
    }

    /// The raw mutual fund instrument CSV: `GET /mf/instruments`.
    pub async fn get_instruments_csv(&self) -> Result<String> {
        self.client.get_raw("/mf/instruments").await
    }

    /// Every fund on the platform, parsed from `GET /mf/instruments`.
    pub async fn get_instruments(&self) -> Result<Vec<MfInstrument>> {
        self.client
            .get_csv("/mf/instruments", parse_instruments)
            .await
    }
}

fn parse_instruments(data: &str) -> std::result::Result<Vec<MfInstrument>, HttpError> {
    let mut rdr = csv::Reader::from_reader(data.as_bytes());
    let mut records = Vec::new();
    for (row, result) in rdr.deserialize().enumerate() {
        let record: MfInstrument = result.map_err(|_| {
            HttpError::new(
                HttpErrorKind::Decode,
                Method::Get,
                Endpoint::MfInstruments,
                TransportStage::ResponseReceived,
            )
            .with_status(200)
            .with_detail(&format!(
                "mutual fund instrument CSV row {} is malformed",
                row + 1
            ))
        })?;
        records.push(record);
    }
    Ok(records)
}
