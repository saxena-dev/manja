//! Connection, reconnect and credential-rejection lifecycle.
//!
//! Every way a connection attempt or an established connection can end has
//! one explicit disposition:
//!
//! | Cause | Disposition |
//! |---|---|
//! | handshake rejected with 401 or 403 | terminal `AuthRejected`: never retried with the same credentials |
//! | handshake rejected with another 4xx | terminal |
//! | handshake rejected with 5xx | retry |
//! | handshake timeout (`B-TK-01`), connect failure | retry |
//! | end of stream without a close frame (`Eof`) | retry |
//! | close frame from the peer (`RemoteClose`) | retry |
//! | stalled peer: no message, heartbeat or ping within `B-TK-04` (`LivenessTimeout`) | retry |
//! | transport or send failure (`TransportError`) | retry |
//! | WebSocket protocol violation or oversized message (`ProtocolError`) | retry |
//! | consumer overload, receiver or handles dropped, shutdown | terminal, never retried |
//!
//! Retries are bounded per outage by attempts and total time (`B-TK-02`),
//! whichever comes first, and then the ticker fails with
//! [`TerminalReason::ReconnectExhausted`]. Between attempts the owner waits
//! a capped, fully jittered exponential backoff (`B-TK-03`); the wait is
//! interrupted by shutdown and keeps accepting commands. There is no other
//! way to read the socket, so nothing bypasses this recovery.
//!
//! Every attempt, failed handshakes included, runs under a fresh
//! [`ConnectionEpoch`](crate::kite::envelope::ConnectionEpoch). A reconnect
//! emits [`GapFacts`](crate::kite::envelope::GapFacts) that state what the
//! owner saw: the previous epoch, its last ingress sequence, and when it
//! ended and the new connection began. Nothing is backfilled and no count of
//! missed messages is claimed. The retained desired subscription map is
//! written to the new connection, subscribe before mode, before `Active` is
//! reported; a failed write is another retryable loss, never `Active`.
//!
//! The ticker never logs in, refreshes or invalidates a token, or calls any
//! HTTP endpoint: it holds the credentials it was built with, and after a
//! rejection it stops.
//!
//! Heartbeats are raw binary observations like any other message; they also
//! keep the liveness timer alive. Liveness is a transport fact, not quote
//! freshness.
//!
use std::time::Duration;

use crate::kite::envelope::DisconnectReason;
use crate::kite::ticker::actor::owner::{TerminalReason, TickerLimitError};

/// Reconnect and liveness bounds (`docs/contract.md` §3.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconnectLimits {
    attempts: u32,
    outage_deadline: Duration,
    backoff_initial: Duration,
    backoff_cap: Duration,
    liveness_timeout: Duration,
    jitter_seed: Option<u64>,
}

impl Default for ReconnectLimits {
    fn default() -> Self {
        Self {
            attempts: 10,
            outage_deadline: Duration::from_secs(300),
            backoff_initial: Duration::from_millis(500),
            backoff_cap: Duration::from_secs(30),
            liveness_timeout: Duration::from_secs(15),
            jitter_seed: None,
        }
    }
}

fn within<T: PartialOrd>(id: &'static str, v: T, min: T, max: T) -> Result<T, TickerLimitError> {
    if v < min || v > max {
        Err(TickerLimitError(id))
    } else {
        Ok(v)
    }
}

impl ReconnectLimits {
    /// Connection attempts per outage (`B-TK-02`, 1 to 100). 1 means no
    /// reconnect.
    pub fn with_attempts(mut self, n: u32) -> Result<Self, TickerLimitError> {
        self.attempts = within("B-TK-02 attempts", n, 1, 100)?;
        Ok(self)
    }

    /// Total time per outage (`B-TK-02`, 10 s to 1 h).
    pub fn with_outage_deadline(mut self, d: Duration) -> Result<Self, TickerLimitError> {
        self.outage_deadline = within(
            "B-TK-02 deadline",
            d,
            Duration::from_secs(10),
            Duration::from_secs(3600),
        )?;
        Ok(self)
    }

    /// Backoff initial delay (50 ms to 10 s) and cap (50 ms to 300 s, at
    /// least the initial delay) (`B-TK-03`).
    pub fn with_backoff(
        mut self,
        initial: Duration,
        cap: Duration,
    ) -> Result<Self, TickerLimitError> {
        self.backoff_initial = within(
            "B-TK-03 initial",
            initial,
            Duration::from_millis(50),
            Duration::from_secs(10),
        )?;
        self.backoff_cap = within(
            "B-TK-03 cap",
            cap,
            Duration::from_millis(50),
            Duration::from_secs(300),
        )?;
        if cap < initial {
            return Err(TickerLimitError("B-TK-03 cap >= initial"));
        }
        Ok(self)
    }

    /// Liveness timeout (`B-TK-04`, 2 s to 120 s).
    pub fn with_liveness_timeout(mut self, d: Duration) -> Result<Self, TickerLimitError> {
        self.liveness_timeout = within(
            "B-TK-04",
            d,
            Duration::from_secs(2),
            Duration::from_secs(120),
        )?;
        Ok(self)
    }

    /// Seed the backoff jitter, for reproducible tests.
    pub fn with_jitter_seed(mut self, seed: u64) -> Self {
        self.jitter_seed = Some(seed);
        self
    }

    /// `B-TK-02` attempts.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
    /// `B-TK-02` deadline.
    pub fn outage_deadline(&self) -> Duration {
        self.outage_deadline
    }
    /// `B-TK-03` initial delay and cap.
    pub fn backoff(&self) -> (Duration, Duration) {
        (self.backoff_initial, self.backoff_cap)
    }
    /// `B-TK-04`.
    pub fn liveness_timeout(&self) -> Duration {
        self.liveness_timeout
    }
}

/// Whether a failure is retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disposition {
    Retry,
    Terminal,
}

/// The disposition of a failed connection attempt.
pub(crate) fn after_handshake(reason: &TerminalReason) -> Disposition {
    match reason {
        TerminalReason::HandshakeTimeout | TerminalReason::ConnectFailed => Disposition::Retry,
        TerminalReason::HandshakeRejected { http_status } if *http_status >= 500 => {
            Disposition::Retry
        }
        _ => Disposition::Terminal,
    }
}

/// The disposition of an established connection that ended.
pub(crate) fn after_disconnect(reason: DisconnectReason) -> Disposition {
    match reason {
        DisconnectReason::Eof
        | DisconnectReason::RemoteClose
        | DisconnectReason::LivenessTimeout
        | DisconnectReason::TransportError
        | DisconnectReason::ProtocolError => Disposition::Retry,
        _ => Disposition::Terminal,
    }
}

// SplitMix64: jitter needs spread, not cryptographic strength.
pub(crate) struct Backoff {
    state: u64,
    initial: Duration,
    cap: Duration,
}

impl Backoff {
    pub(crate) fn new(limits: &ReconnectLimits) -> Self {
        let seed = limits.jitter_seed.unwrap_or_else(|| {
            use std::hash::{BuildHasher, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u8(0);
            h.finish()
        });
        Self {
            state: seed,
            initial: limits.backoff_initial,
            cap: limits.backoff_cap,
        }
    }

    /// A delay in `[0, min(cap, initial × 2^(failures - 1))]`.
    pub(crate) fn delay(&mut self, failures: u32) -> Duration {
        let exp = self
            .initial
            .saturating_mul(1u32 << failures.saturating_sub(1).min(16))
            .min(self.cap);
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let nanos = exp.as_nanos() as u64;
        Duration::from_nanos(z % (nanos + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cause_has_a_disposition() {
        use DisconnectReason::*;
        for r in [
            Eof,
            RemoteClose,
            LivenessTimeout,
            TransportError,
            ProtocolError,
        ] {
            assert_eq!(after_disconnect(r), Disposition::Retry, "{r:?}");
        }
        for r in [DeliveryOverload, Shutdown] {
            assert_eq!(after_disconnect(r), Disposition::Terminal, "{r:?}");
        }
        let cases = [
            (
                TerminalReason::AuthRejected { http_status: 401 },
                Disposition::Terminal,
            ),
            (
                TerminalReason::AuthRejected { http_status: 403 },
                Disposition::Terminal,
            ),
            (
                TerminalReason::HandshakeRejected { http_status: 400 },
                Disposition::Terminal,
            ),
            (
                TerminalReason::HandshakeRejected { http_status: 429 },
                Disposition::Terminal,
            ),
            (
                TerminalReason::HandshakeRejected { http_status: 503 },
                Disposition::Retry,
            ),
            (TerminalReason::HandshakeTimeout, Disposition::Retry),
            (TerminalReason::ConnectFailed, Disposition::Retry),
        ];
        for (r, d) in cases {
            assert_eq!(after_handshake(&r), d, "{r:?}");
        }
    }

    #[test]
    fn backoff_is_capped_jittered_and_reproducible() {
        let limits = ReconnectLimits::default()
            .with_backoff(Duration::from_millis(100), Duration::from_millis(400))
            .unwrap()
            .with_jitter_seed(9);
        let (mut a, mut b) = (Backoff::new(&limits), Backoff::new(&limits));
        let mut seen = std::collections::BTreeSet::new();
        for n in 1..=20 {
            let d = a.delay(n);
            assert_eq!(d, b.delay(n));
            let bound = Duration::from_millis(100 * (1 << (n - 1).min(2)));
            assert!(d <= bound.min(Duration::from_millis(400)), "{n}: {d:?}");
            seen.insert(d);
        }
        assert!(seen.len() > 10, "jittered");
    }

    #[test]
    fn bounds_are_validated() {
        let d = ReconnectLimits::default();
        assert!(d.clone().with_attempts(0).is_err());
        assert!(d.clone().with_attempts(101).is_err());
        assert!(d
            .clone()
            .with_outage_deadline(Duration::from_secs(9))
            .is_err());
        assert!(d
            .clone()
            .with_backoff(Duration::from_millis(49), Duration::from_secs(1))
            .is_err());
        assert!(d
            .clone()
            .with_backoff(Duration::from_secs(2), Duration::from_secs(1))
            .is_err());
        assert!(d.with_liveness_timeout(Duration::from_secs(1)).is_err());
    }
}
