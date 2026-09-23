//! Desired subscriptions and command revisions.
//!
//! The owner keeps one canonical map, [`InstrumentToken`] to [`Mode`], and
//! every command operates on it:
//!
//! | Command | Effect |
//! |---|---|
//! | [`SubscriptionCommand::Subscribe`] | desire each token in the given mode; a token already desired takes the new mode |
//! | [`SubscriptionCommand::Unsubscribe`] | stop desiring each token; a token not desired is ignored |
//! | [`SubscriptionCommand::SetMode`] | change the mode of desired tokens; a token not desired is an error, never an implicit subscription |
//! | [`SubscriptionCommand::Replace`] | replace the whole map atomically; a token listed with two modes is an error |
//!
//! A command either succeeds whole or changes nothing. A command that
//! changes the map is assigned the next [`Revision`]; one that changes
//! nothing, such as a repeated identical subscribe, keeps the current
//! revision and sends nothing. More than `B-TK-11` desired tokens is an
//! error. That bound is per connection (`kite-api-docs/docs/connect/v3/websocket.md:7`);
//! the limit of three connections per API key is not enforced by one ticker
//! and must be coordinated by the caller.
//!
//! # Completion is acceptance
//!
//! A command completes with `Ok(Revision)` once the owner has validated it,
//! updated the map and assigned the revision. That is **not** broker
//! acknowledgement: the protocol has none. Lifecycle events then report
//! each revision as `CommandsSent` (written to the socket, not confirmed),
//! `Superseded` (replaced before it was sent) or `SendFailed`. `Active`
//! says that the desired map was written to the connection; it says nothing
//! about quote freshness or readiness to trade.
//!
//! # Wire order
//!
//! Reconciling the connection from what it was last sent to the desired map
//! produces, deterministically: one `unsubscribe` of the removed tokens,
//! one `subscribe` of the added tokens, then one `mode` request per mode in
//! the fixed order LTP, quote, full, for every token that was added or
//! changed mode. Tokens are ascending within each request. Every added
//! token gets an explicit mode, so no broker default is relied on.
//!
use std::collections::BTreeMap;
use std::fmt;

use crate::kite::protocol::InstrumentToken;
use crate::kite::ticker::models::{Mode, TickerRequest};

/// The largest `B-TK-11`: instruments per connection
/// (`kite-api-docs/docs/connect/v3/websocket.md:7`).
pub const MAX_INSTRUMENTS_PER_CONNECTION: usize = 3000;

/// A desired-state revision. 0 is the initial, empty map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);

/// A command on the desired map.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubscriptionCommand {
    /// Desire `tokens` in `mode`.
    Subscribe {
        /// Tokens to desire.
        tokens: Vec<InstrumentToken>,
        /// Their mode.
        mode: Mode,
    },
    /// Stop desiring `tokens`.
    Unsubscribe {
        /// Tokens to drop.
        tokens: Vec<InstrumentToken>,
    },
    /// Change the mode of desired `tokens`.
    SetMode {
        /// Desired tokens.
        tokens: Vec<InstrumentToken>,
        /// Their new mode.
        mode: Mode,
    },
    /// Replace the whole map.
    Replace {
        /// The new map, as pairs.
        desired: Vec<(InstrumentToken, Mode)>,
    },
}

/// Why a command was refused. The map is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubscriptionError {
    /// A subscribe, unsubscribe or set-mode named no token.
    Empty,
    /// A replacement listed `token` with two different modes.
    ConflictingModes {
        /// The token.
        token: InstrumentToken,
    },
    /// Set-mode named `token`, which is not desired.
    NotDesired {
        /// The token.
        token: InstrumentToken,
    },
    /// The map would hold `requested` tokens, over the per-connection
    /// bound `max` (`B-TK-11`).
    CapacityExceeded {
        /// Tokens the command would leave desired.
        requested: usize,
        /// The bound.
        max: usize,
    },
}

impl fmt::Display for SubscriptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the command names no instrument"),
            Self::ConflictingModes { token } => {
                write!(f, "token {} is listed with two modes", token.get())
            }
            Self::NotDesired { token } => write!(
                f,
                "token {} is not subscribed; set-mode never subscribes",
                token.get()
            ),
            Self::CapacityExceeded { requested, max } => write!(
                f,
                "{requested} instruments exceed the per-connection bound of {max}"
            ),
        }
    }
}

impl std::error::Error for SubscriptionError {}

/// The canonical desired map and its revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesiredSubscriptions {
    map: BTreeMap<InstrumentToken, Mode>,
    revision: Revision,
    capacity: usize,
}

impl DesiredSubscriptions {
    /// An empty map at revision 0, holding at most `capacity` tokens
    /// (clamped to [`MAX_INSTRUMENTS_PER_CONNECTION`]).
    pub fn new(capacity: usize) -> Self {
        Self {
            map: BTreeMap::new(),
            revision: Revision(0),
            capacity: capacity.clamp(1, MAX_INSTRUMENTS_PER_CONNECTION),
        }
    }

    /// The current revision.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The desired map.
    pub fn map(&self) -> &BTreeMap<InstrumentToken, Mode> {
        &self.map
    }

    /// Apply `command` whole or not at all. Returns the revision in effect
    /// afterwards, and whether the map changed.
    pub fn apply(
        &mut self,
        command: &SubscriptionCommand,
    ) -> Result<(Revision, bool), SubscriptionError> {
        let mut next = self.map.clone();
        match command {
            SubscriptionCommand::Subscribe { tokens, mode } => {
                if tokens.is_empty() {
                    return Err(SubscriptionError::Empty);
                }
                for t in tokens {
                    next.insert(*t, mode.clone());
                }
            }
            SubscriptionCommand::Unsubscribe { tokens } => {
                if tokens.is_empty() {
                    return Err(SubscriptionError::Empty);
                }
                for t in tokens {
                    next.remove(t);
                }
            }
            SubscriptionCommand::SetMode { tokens, mode } => {
                if tokens.is_empty() {
                    return Err(SubscriptionError::Empty);
                }
                for t in tokens {
                    match next.get_mut(t) {
                        Some(m) => *m = mode.clone(),
                        None => return Err(SubscriptionError::NotDesired { token: *t }),
                    }
                }
            }
            SubscriptionCommand::Replace { desired } => {
                next.clear();
                for (t, mode) in desired {
                    if let Some(previous) = next.insert(*t, mode.clone()) {
                        if previous != *mode {
                            return Err(SubscriptionError::ConflictingModes { token: *t });
                        }
                    }
                }
            }
        }
        if next.len() > self.capacity {
            return Err(SubscriptionError::CapacityExceeded {
                requested: next.len(),
                max: self.capacity,
            });
        }
        if next == self.map {
            return Ok((self.revision, false));
        }
        self.map = next;
        self.revision = Revision(self.revision.0 + 1);
        Ok((self.revision, true))
    }
}

/// The requests that take a connection from `sent` to `desired`, in wire
/// order (see the module docs). Empty when they are equal.
pub fn reconcile(
    sent: &BTreeMap<InstrumentToken, Mode>,
    desired: &BTreeMap<InstrumentToken, Mode>,
) -> Vec<TickerRequest> {
    let raw = |tokens: Vec<InstrumentToken>| tokens.into_iter().map(u32::from).collect();
    let removed: Vec<_> = sent
        .keys()
        .filter(|t| !desired.contains_key(t))
        .copied()
        .collect();
    let added: Vec<_> = desired
        .keys()
        .filter(|t| !sent.contains_key(t))
        .copied()
        .collect();
    let mut out = Vec::new();
    if !removed.is_empty() {
        out.push(TickerRequest::unsubscribe(raw(removed)));
    }
    if !added.is_empty() {
        out.push(TickerRequest::subscribe(raw(added)));
    }
    for mode in Mode::WIRE_ORDER {
        let tokens: Vec<_> = desired
            .iter()
            .filter(|(t, m)| **m == mode && sent.get(t) != Some(m))
            .map(|(t, _)| *t)
            .collect();
        if !tokens.is_empty() {
            out.push(TickerRequest::mode(mode, raw(tokens)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(v: u32) -> InstrumentToken {
        InstrumentToken::new(v)
    }

    fn wire(requests: &[TickerRequest]) -> Vec<String> {
        requests.iter().map(ToString::to_string).collect()
    }

    fn sub(tokens: &[u32], mode: Mode) -> SubscriptionCommand {
        SubscriptionCommand::Subscribe {
            tokens: tokens.iter().copied().map(t).collect(),
            mode,
        }
    }

    #[test]
    fn identical_subscribes_are_idempotent_and_send_nothing_more() {
        let mut d = DesiredSubscriptions::new(3000);
        assert_eq!(d.apply(&sub(&[1, 2], Mode::Full)), Ok((Revision(1), true)));
        let sent = d.map().clone();
        assert_eq!(d.apply(&sub(&[2, 1], Mode::Full)), Ok((Revision(1), false)));
        assert!(reconcile(&sent, d.map()).is_empty());
    }

    #[test]
    fn refused_commands_change_nothing() {
        let mut d = DesiredSubscriptions::new(3);
        d.apply(&sub(&[1, 2], Mode::Quote)).unwrap();
        let before = d.clone();
        let refusals = [
            (
                SubscriptionCommand::SetMode {
                    tokens: vec![t(1), t(9)],
                    mode: Mode::Full,
                },
                SubscriptionError::NotDesired { token: t(9) },
            ),
            (
                SubscriptionCommand::Replace {
                    desired: vec![(t(5), Mode::LTP), (t(6), Mode::Full), (t(5), Mode::Full)],
                },
                SubscriptionError::ConflictingModes { token: t(5) },
            ),
            (
                sub(&[3, 4], Mode::LTP),
                SubscriptionError::CapacityExceeded {
                    requested: 4,
                    max: 3,
                },
            ),
            (sub(&[], Mode::LTP), SubscriptionError::Empty),
            (
                SubscriptionCommand::Unsubscribe { tokens: vec![] },
                SubscriptionError::Empty,
            ),
        ];
        for (command, error) in refusals {
            assert_eq!(d.apply(&command), Err(error));
            assert_eq!(d, before, "{command:?}");
        }
        // At the bound exactly, and an identical duplicate in a replacement.
        assert_eq!(
            d.apply(&SubscriptionCommand::Replace {
                desired: vec![
                    (t(7), Mode::LTP),
                    (t(8), Mode::LTP),
                    (t(9), Mode::LTP),
                    (t(7), Mode::LTP)
                ],
            }),
            Ok((Revision(2), true))
        );
    }

    #[test]
    fn the_per_connection_bound_is_3000() {
        let mut d = DesiredSubscriptions::new(usize::MAX);
        let all: Vec<u32> = (1..=3000).collect();
        assert!(d.apply(&sub(&all, Mode::LTP)).is_ok());
        assert_eq!(
            d.apply(&sub(&[3001], Mode::LTP)),
            Err(SubscriptionError::CapacityExceeded {
                requested: 3001,
                max: 3000
            })
        );
        assert_eq!(d.map().len(), 3000);
    }

    #[test]
    fn reconciliation_is_ordered_and_deterministic() {
        let sent: BTreeMap<_, _> = [(t(5), Mode::Quote), (t(1), Mode::Full), (t(3), Mode::LTP)]
            .into_iter()
            .collect();
        let desired: BTreeMap<_, _> = [
            (t(9), Mode::Full),
            (t(1), Mode::LTP),
            (t(7), Mode::Quote),
            (t(3), Mode::LTP),
            (t(2), Mode::Full),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            wire(&reconcile(&sent, &desired)),
            [
                r#"{"a":"unsubscribe","v":[5]}"#,
                r#"{"a":"subscribe","v":[2,7,9]}"#,
                r#"{"a":"mode","v":["ltp",[1]]}"#,
                r#"{"a":"mode","v":["quote",[7]]}"#,
                r#"{"a":"mode","v":["full",[2,9]]}"#,
            ]
        );
        // From nothing: subscribe everything, then every mode explicitly.
        assert_eq!(
            wire(&reconcile(&BTreeMap::new(), &desired)),
            [
                r#"{"a":"subscribe","v":[1,2,3,7,9]}"#,
                r#"{"a":"mode","v":["ltp",[1,3]]}"#,
                r#"{"a":"mode","v":["quote",[7]]}"#,
                r#"{"a":"mode","v":["full",[2,9]]}"#,
            ]
        );
    }

    // A model of the broker's view of one connection, driven by the wire
    // requests: subscribe adds tokens with some mode, mode changes it,
    // unsubscribe removes.
    fn broker_view(
        requests: &[TickerRequest],
        start: &BTreeMap<InstrumentToken, Mode>,
    ) -> BTreeMap<InstrumentToken, Mode> {
        let mut view = start.clone();
        for r in requests {
            let v: serde_json::Value = serde_json::from_str(&r.to_string()).unwrap();
            let tokens = |x: &serde_json::Value| -> Vec<InstrumentToken> {
                x.as_array()
                    .unwrap()
                    .iter()
                    .map(|n| t(n.as_u64().unwrap() as u32))
                    .collect()
            };
            match v["a"].as_str().unwrap() {
                "subscribe" => {
                    for tok in tokens(&v["v"]) {
                        // Not yet in a known mode until a mode request.
                        view.entry(tok).or_insert(Mode::Quote);
                    }
                }
                "unsubscribe" => {
                    for tok in tokens(&v["v"]) {
                        view.remove(&tok);
                    }
                }
                "mode" => {
                    let mode: Mode = serde_json::from_value(v["v"][0].clone()).unwrap();
                    for tok in tokens(&v["v"][1]) {
                        assert!(view.contains_key(&tok), "mode before subscribe");
                        view.insert(tok, mode.clone());
                    }
                }
                other => panic!("{other}"),
            }
        }
        view
    }

    #[test]
    fn random_command_sequences_keep_every_invariant() {
        // A seeded linear congruential generator: reproducible, no
        // dependency.
        let mut seed: u64 = 0x5eed;
        let mut next = move |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let modes = [Mode::LTP, Mode::Quote, Mode::Full];
        for _ in 0..200 {
            let mut d = DesiredSubscriptions::new(12);
            let mut sent = BTreeMap::new();
            let mut last_revision = Revision(0);
            for _ in 0..40 {
                let tokens: Vec<InstrumentToken> =
                    (0..next(5)).map(|_| t(next(20) as u32)).collect();
                let mode = modes[next(3) as usize].clone();
                let command = match next(4) {
                    0 => SubscriptionCommand::Subscribe { tokens, mode },
                    1 => SubscriptionCommand::Unsubscribe { tokens },
                    2 => SubscriptionCommand::SetMode { tokens, mode },
                    _ => SubscriptionCommand::Replace {
                        desired: tokens
                            .into_iter()
                            .map(|t| (t, modes[next(3) as usize].clone()))
                            .collect(),
                    },
                };
                let before = d.clone();
                match d.apply(&command) {
                    Ok((revision, changed)) => {
                        assert_eq!(changed, revision > last_revision);
                        assert_eq!(changed, before.map() != d.map());
                        last_revision = revision;
                    }
                    Err(_) => assert_eq!(d, before),
                }
                assert!(d.map().len() <= 12);
                // Sometimes the connection catches up.
                if next(2) == 0 {
                    let requests = reconcile(&sent, d.map());
                    assert_eq!(&broker_view(&requests, &sent), d.map());
                    assert!(requests.len() <= 5, "one request per kind and mode");
                    sent = d.map().clone();
                    assert!(reconcile(&sent, d.map()).is_empty());
                }
            }
        }
    }
}
