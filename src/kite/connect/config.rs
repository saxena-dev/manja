//! Configuration for the asynchronous HTTP client.
//!
//! [`Config`] holds the API base URL and the runtime limits.
//!
//! [`HttpLimits`] holds the validated runtime bounds of the SDK contract
//! (`B-HTTP-*`). Every bound has a default, a minimum and a maximum; building
//! limits outside those ranges is a configuration error, and no bound means
//! "unlimited".
//!
//! Configuration is always explicit. Nothing here reads environment
//! variables: [`Config::default`] is the documented production endpoint, and
//! [`Config::new`] takes the base URL from the caller.
//!
use crate::kite::connect::scheduler::SchedulerLimits;
use crate::kite::traits::KiteConfig;

/// Default v3 API base url.
///
pub const KITECONNECT_API_BASE: &str = "https://api.kite.trade";

/// The HTTP client configuration: API base URL and runtime limits.
#[derive(Clone, Debug)]
pub struct Config {
    /// Base URL for the KiteConnect API.
    api_base: String,
    /// Runtime bounds.
    limits: HttpLimits,
}

/// A bound was outside its permitted range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitError {
    /// Contract identifier of the bound, e.g. `B-HTTP-08`.
    pub bound: &'static str,
    /// Human-readable permitted range.
    pub range: String,
}

impl std::fmt::Display for LimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} must be in {}", self.bound, self.range)
    }
}

impl std::error::Error for LimitError {}

fn check_range<T: PartialOrd + std::fmt::Debug>(
    bound: &'static str,
    value: T,
    min: T,
    max: T,
) -> Result<T, LimitError> {
    if value < min || value > max {
        Err(LimitError {
            bound,
            range: format!("{min:?}..={max:?}"),
        })
    } else {
        Ok(value)
    }
}

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

/// Validated HTTP runtime bounds (SDK contract §5.2).
///
/// | Bound | Default | Range |
/// |---|---|---|
/// | `B-HTTP-08` JSON response body | 4 MiB | 64 KiB ..= 64 MiB |
/// | `B-HTTP-08` instrument CSV body | 64 MiB | 1 MiB ..= 256 MiB |
/// | `B-HTTP-09` request body | 1 MiB | 1 KiB ..= 8 MiB |
/// | `B-HTTP-10` in-flight attempts per transport | 32 | 1 ..= 256 |
///
/// Deadline, attempt and retry bounds (`B-HTTP-01`–`05`, `-11`, `-12`) are
/// the [`SchedulerLimits`]. Admission bounds (`B-HTTP-06`, `B-HTTP-07`)
/// belong to the shared
/// [`Admission`](crate::kite::connect::admission::Admission) scope.
///
/// A response larger than its bound is a `Decode` error that preserves the
/// HTTP status; the body is not buffered past the bound. The kite-api-docs
/// snapshot gives no size for the instrument dump
/// (`docs/connect/v3/market-quotes.md:17,52`); 64 MiB is roughly 300 000
/// rows of 200 bytes, several times the official sample's row width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpLimits {
    json_body_bytes: usize,
    csv_body_bytes: usize,
    request_body_bytes: usize,
    in_flight: usize,
    scheduler: SchedulerLimits,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            json_body_bytes: 4 * MIB,
            csv_body_bytes: 64 * MIB,
            request_body_bytes: MIB,
            in_flight: 32,
            scheduler: SchedulerLimits::default(),
        }
    }
}

impl HttpLimits {
    /// Maximum JSON response body, in bytes.
    pub fn json_body_bytes(&self) -> usize {
        self.json_body_bytes
    }

    /// Maximum instrument CSV response body, in bytes.
    pub fn csv_body_bytes(&self) -> usize {
        self.csv_body_bytes
    }

    /// Set the JSON body bound (`B-HTTP-08`).
    pub fn with_json_body_bytes(mut self, bytes: usize) -> Result<Self, LimitError> {
        self.json_body_bytes = check_range("B-HTTP-08 (JSON)", bytes, 64 * KIB, 64 * MIB)?;
        Ok(self)
    }

    /// Deadline, attempt-timeout, retry and permit bounds.
    pub fn scheduler(&self) -> &SchedulerLimits {
        &self.scheduler
    }

    /// Replace the deadline, attempt-timeout, retry and permit bounds.
    pub fn with_scheduler(mut self, scheduler: SchedulerLimits) -> Self {
        self.scheduler = scheduler;
        self
    }

    /// Maximum request body, in bytes. A larger request is a `Validation`
    /// error before admission.
    pub fn request_body_bytes(&self) -> usize {
        self.request_body_bytes
    }

    /// Set the request body bound (`B-HTTP-09`).
    pub fn with_request_body_bytes(mut self, bytes: usize) -> Result<Self, LimitError> {
        self.request_body_bytes = check_range("B-HTTP-09", bytes, KIB, 8 * MIB)?;
        Ok(self)
    }

    /// Maximum concurrent transport attempts on one transport.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Set the in-flight attempt bound (`B-HTTP-10`).
    pub fn with_in_flight(mut self, attempts: usize) -> Result<Self, LimitError> {
        self.in_flight = check_range("B-HTTP-10", attempts, 1, 256)?;
        Ok(self)
    }

    /// Set the instrument CSV body bound (`B-HTTP-08`).
    pub fn with_csv_body_bytes(mut self, bytes: usize) -> Result<Self, LimitError> {
        self.csv_body_bytes = check_range("B-HTTP-08 (CSV)", bytes, MIB, 256 * MIB)?;
        Ok(self)
    }
}

impl Default for Config {
    /// The documented production endpoint. No environment variable is
    /// consulted.
    fn default() -> Self {
        Self::new(KITECONNECT_API_BASE)
    }
}

impl KiteConfig for Config {
    /// The URL of `path`, which starts with a slash.
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    fn api_base(&self) -> &str {
        self.api_base.as_str()
    }
}

impl Config {
    /// A configuration for the API at `api_base`, with default limits.
    pub fn new(api_base: impl Into<String>) -> Self {
        Self {
            api_base: api_base.into(),
            limits: HttpLimits::default(),
        }
    }

    /// Replace the runtime bounds.
    pub fn with_limits(mut self, limits: HttpLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The runtime bounds.
    pub fn limits(&self) -> &HttpLimits {
        &self.limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_reads_no_environment() {
        std::env::set_var("KITECONNECT_API_BASE", "http://sentinel.invalid");
        let config = Config::default();
        assert_eq!(config.api_base(), KITECONNECT_API_BASE);
        assert_eq!(Config::new("http://x").url("/a"), "http://x/a");
    }

    #[test]
    fn body_bounds_are_validated() {
        let d = HttpLimits::default();
        assert_eq!(
            (d.json_body_bytes(), d.csv_body_bytes()),
            (4 * MIB, 64 * MIB)
        );
        assert!(d.clone().with_json_body_bytes(64 * KIB).is_ok());
        assert!(d.clone().with_json_body_bytes(64 * MIB).is_ok());
        assert!(d.clone().with_json_body_bytes(64 * KIB - 1).is_err());
        assert!(d.clone().with_json_body_bytes(64 * MIB + 1).is_err());
        assert!(d.clone().with_csv_body_bytes(MIB).is_ok());
        assert!(d.clone().with_csv_body_bytes(256 * MIB).is_ok());
        assert!(d.clone().with_csv_body_bytes(MIB - 1).is_err());
        assert_eq!(d.request_body_bytes(), MIB);
        assert!(d.clone().with_request_body_bytes(KIB).is_ok());
        assert!(d.clone().with_request_body_bytes(8 * MIB).is_ok());
        assert!(d.clone().with_request_body_bytes(KIB - 1).is_err());
        assert!(d.clone().with_request_body_bytes(8 * MIB + 1).is_err());
        assert_eq!(d.in_flight(), 32);
        assert!(d.clone().with_in_flight(1).is_ok());
        assert!(d.clone().with_in_flight(256).is_ok());
        assert!(d.clone().with_in_flight(0).is_err());
        assert!(d.clone().with_in_flight(257).is_err());
        let err = d.with_csv_body_bytes(256 * MIB + 1).unwrap_err();
        assert_eq!(err.bound, "B-HTTP-08 (CSV)");
    }
}
