//! Construction reads no environment variable.
//!
//! A credentials snapshot holds only what the caller passed, and
//! `Config::default()` is the production endpoint whatever the process
//! environment holds. Setting a variable is only sound while no other thread
//! reads the environment, so this is the only test in its binary.

use manja::kite::connect::credentials::Credentials;

#[test]
fn construction_ignores_the_environment() {
    // SAFETY: the only test in this binary. The harness reads its own
    // variables before running it, and nothing else in the process reads
    // or writes the environment meanwhile.
    unsafe {
        std::env::set_var("KITECONNECT_API_KEY", "SENTINEL-env-key");
        std::env::set_var("KITECONNECT_API_BASE", "http://sentinel.invalid");
    }
    let creds = Credentials::new("explicit", "token").unwrap();
    assert_eq!(creds.api_key().as_str(), "explicit");
    #[cfg(feature = "http")]
    {
        use manja::kite::connect::config::{Config, KITECONNECT_API_BASE};
        assert_eq!(Config::default().api_base(), KITECONNECT_API_BASE);
    }
}
