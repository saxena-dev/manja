use crate::kite::connect::{
    api::{
        Alerts, Charges, Gtt, Historical, Market, Margins, MutualFunds, Orders, Portfolio,
        Session, User,
    },
    client::HTTPClient,
    config::Config,
};

/// Top-level asynchronous client for the Kite Connect API.
///
/// `ManjaClient` is a facade over the lower-level `HTTPClient` type and
/// provides convenient constructors and domain-specific accessors
/// (`user`, `session`, `orders`, `portfolio`, `market`, `margins`, `charges`).
///
/// In most cases, users should prefer `ManjaClient` over using
/// `kite::connect::client::HTTPClient` directly.
#[derive(Clone)]
pub struct ManjaClient {
    http: HTTPClient,
}

impl ManjaClient {
    /// Create a new client from an explicit HTTP configuration.
    ///
    /// This constructor does not read from the environment; it uses the
    /// provided `Config` value directly.
    pub fn new(config: Config) -> Self {
        Self {
            http: HTTPClient::with_config(config),
        }
    }

    /// Create a new client using configuration loaded from the environment.
    ///
    /// This uses the same defaults as `HTTPClient::default()`, including any
    /// relevant environment variables (such as `KITECONNECT_API_BASE`).
    pub fn from_env() -> Self {
        Self {
            http: HTTPClient::default(),
        }
    }

    /// Access the underlying HTTP client.
    pub fn http(&self) -> &HTTPClient {
        &self.http
    }

    /// Mutable access to the underlying HTTP client.
    pub fn http_mut(&mut self) -> &mut HTTPClient {
        &mut self.http
    }

    /// Access user-related HTTP APIs.
    pub fn user(&self) -> User<'_> {
        self.http.user()
    }

    /// Access session-related HTTP APIs.
    pub fn session(&mut self) -> Session<'_> {
        self.http.session()
    }

    /// Access order-related HTTP APIs.
    pub fn orders(&mut self) -> Orders<'_> {
        self.http.orders()
    }

    /// Access portfolio-related HTTP APIs.
    pub fn portfolio(&self) -> Portfolio<'_> {
        Portfolio::new(self.http())
    }

    /// Access market data HTTP APIs.
    pub fn market(&mut self) -> Market<'_> {
        self.http.market()
    }

    /// Access historical data HTTP APIs.
    pub fn historical(&mut self) -> Historical<'_> {
        self.http.historical()
    }

    /// Access margins-related HTTP APIs.
    pub fn margins(&mut self) -> Margins<'_> {
        self.http.margins()
    }

    /// Access charges-related HTTP APIs.
    pub fn charges(&mut self) -> Charges<'_> {
        self.http.charges()
    }

    /// Access GTT-related HTTP APIs.
    pub fn gtt(&mut self) -> Gtt<'_> {
        self.http.gtt()
    }

    /// Access Alerts-related HTTP APIs.
    pub fn alerts(&mut self) -> Alerts<'_> {
        self.http.alerts()
    }

    /// Access Mutual Funds-related HTTP APIs.
    pub fn mutual_funds(&mut self) -> MutualFunds<'_> {
        self.http.mutual_funds()
    }
}
