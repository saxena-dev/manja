//! Portfolio API group: `/portfolio/`
//!
//! This module provides functionality to interact with the portfolio-related
//! endpoints of Kite Connect API.
//!
//! A user's portfolio consists of long term equity holdings and short term
//! positions. The portfolio APIs return instruments in a portfolio with up-to-date
//! profit and loss computations.
//!
//! Refer to the official API documentation (`kite:portfolio.md`).
//!

use crate::kite::connect::{
    client::HTTPClient,
    models::{
        Auction, Holding, HoldingsAuthorisation, HoldingsAuthorisationRequest, KiteApiResponse,
        PositionConversionRequest, Positions,
    },
    scheduler::DispatchPermit,
};
use crate::kite::error::Result;

/// Holdings, positions, position conversion, holdings auctions and holdings
/// authorisation. Borrowed from a client with
/// [`HTTPClient::portfolio`](crate::kite::connect::client::HTTPClient::portfolio).
///
/// Holdings are long-term equity in the DEMAT account; positions are the
/// day's and carried-forward trades, with profit and loss as Kite computes
/// it.
///
/// # Example
///
/// ```no_run
/// use manja::kite::connect::client::HTTPClient;
/// use manja::kite::connect::config::Config;
/// use manja::kite::connect::credentials::Credentials;
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let client = HTTPClient::new(Config::default())?
///     .with_credentials(Credentials::new("api_key", "access_token")?);
/// let holdings = client.portfolio().get_holdings().await?.data.unwrap_or_default();
/// let positions = client.portfolio().get_positions().await?.data.expect("data");
/// println!("{} holdings, {} net positions", holdings.len(), positions.net.len());
/// # Ok(()) }
/// ```
pub struct Portfolio<'c> {
    /// Reference to the HTTP client used for making API requests.
    pub client: &'c HTTPClient,
}

impl<'c> Portfolio<'c> {
    /// Creates a new instance of `Portfolio` with default API rate limits.
    ///
    /// # Arguments
    ///
    /// * `client` - A reference to the `HTTPClient` used for making API requests.
    ///
    /// # Returns
    ///
    /// A new instance of `Portfolio`.
    ///
    pub fn new(client: &'c HTTPClient) -> Self {
        Self { client }
    }

    // ===== [ KiteConnect API endpoints ] =====

    /// Retrieve the list of long term equity holdings.
    ///
    /// Holdings contain the user's portfolio of long term equity delivery
    /// stocks. An instrument in a holdings portfolio remains there
    /// indefinitely until its sold or is delisted or changed by the exchanges.
    /// Underneath it all, instruments in the holdings reside in the user's
    /// DEMAT account, as settled by exchanges and clearing institutions.
    ///
    pub async fn get_holdings(&self) -> Result<KiteApiResponse<Vec<Holding>>> {
        self.client.get("/portfolio/holdings").await
    }

    /// Retrieve the list of short term positions.
    ///
    /// Positions contain the user's portfolio of short to medium term derivatives
    /// (futures and options contracts) and intraday equity stocks. Instruments
    /// in the positions portfolio remain there until they're sold, or until
    /// expiry, which, for derivatives, is typically three months. Equity positions
    /// carried overnight move to the holdings portfolio the next day.
    ///
    /// The positions API returns two sets of positions, `net` and `day`. `net`
    /// is the actual, current net position portfolio, while `day` is a snapshot
    /// of the buying and selling activity for that particular day. This is
    /// useful for computing intraday profits and losses for trading strategies.
    ///
    pub async fn get_positions(&self) -> Result<KiteApiResponse<Positions>> {
        self.client.get("/portfolio/positions").await
    }

    /// Convert the margin product of an open position:
    /// `PUT /portfolio/positions`, form-encoded, one attempt.
    ///
    /// The request is validated first; an invalid one sends nothing. The
    /// `true` result is the broker's response.
    pub async fn convert_position(
        &self,
        request: &PositionConversionRequest,
    ) -> Result<KiteApiResponse<bool>> {
        self.convert(request, None).await
    }

    /// [`Self::convert_position`] with admitted capacity from
    /// `HTTPClient::admit(PermitTarget::ConvertPosition)`.
    pub async fn convert_position_with_permit(
        &self,
        request: &PositionConversionRequest,
        permit: DispatchPermit,
    ) -> Result<KiteApiResponse<bool>> {
        self.convert(request, Some(permit)).await
    }

    async fn convert(
        &self,
        request: &PositionConversionRequest,
        permit: Option<DispatchPermit>,
    ) -> Result<KiteApiResponse<bool>> {
        self.client
            .send_form(
                reqwest::Method::PUT,
                "/portfolio/positions",
                request.validate(),
                request.form_pairs(),
                permit,
            )
            .await
    }

    /// Retrieve the list of auctions that are currently being held.
    ///
    /// This API returns a list of auctions that are currently being held,
    /// along with details about each auction such as the auction number,
    /// the security being auctioned, the last price of the security, and
    /// the quantity of the security being offered. Only the stocks that
    /// you hold in your demat account will be shown in the auctions list.
    ///
    pub async fn get_auctions(&self) -> Result<KiteApiResponse<Vec<Auction>>> {
        self.client.get("/portfolio/holdings/auctions").await
    }

    /// Start a depository authorisation for selling holdings:
    /// `POST /portfolio/holdings/authorise`, form-encoded, one attempt
    /// (`kite:portfolio.md:503-541`).
    ///
    /// A sell order that needs authorisation fails with HTTP 428
    /// (`kite:portfolio.md:512`; see `HttpError::requires_holdings_authorisation`).
    /// Send the user to [`HoldingsAuthorisation::portal_url`] to key in their
    /// demat PIN, then retry the order. The SDK opens no browser and does not
    /// retry anything. An ISIN that is not 12 ASCII uppercase letters or
    /// digits is a `Validation` error, and nothing is sent.
    pub async fn authorise_holdings(
        &self,
        request: &HoldingsAuthorisationRequest,
    ) -> Result<KiteApiResponse<HoldingsAuthorisation>> {
        self.client
            .send_form(
                reqwest::Method::POST,
                "/portfolio/holdings/authorise",
                request.validate(),
                request.form_pairs(),
                None,
            )
            .await
    }
}
