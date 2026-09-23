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
    models::{Auction, Holding, KiteApiResponse, PositionConversionRequest, Positions},
    scheduler::DispatchPermit,
};
use crate::kite::error::Result;

/// A user's portfolio consists of long term equity holdings and short term
/// positions. The portfolio APIs return instruments in a portfolio with
/// up-to-date profit and loss computations.
///
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

    // TODO!
    // Initiating authorisation
    //
    // curl --request POST https://api.kite.trade/portfolio/holdings/authorise
    // -H "X-Kite-Version: 3" \
    // -H "Authorization: token api_key:access_token" \
    // -d "isin=INE002A01018" -d "quantity=50" \
    // -d "isin=INE009A01021" -d "quantity=50"
    //
    // {
    // "status": "success",
    // "data": {
    // "request_id": "na8QgCeQm05UHG6NL9sAGRzdfSF64UdB"
    // }
    // }
}
