//! Bounded reconnect, liveness and credential rejection,
//! against the loopback WebSocket harness.
//!
//! These run on the real clock with the smallest documented bounds
//! (`B-TK-02` to `B-TK-04`): the owner's timers race real loopback I/O, and
//! a paused Tokio clock would auto-advance them while that I/O is pending.
//! The only network peer is the harness; this test crate does not enable
//! `http`, so no login or session endpoint is reachable.

mod support;

use std::time::Duration;

use futures_util::StreamExt;
use manja::kite::connect::credentials::Credentials;
use manja::kite::envelope::{ConnectionEpoch, DisconnectReason, LifecycleKind};
use manja::kite::protocol::InstrumentToken;
use manja::kite::ticker::actor::lifecycle::ReconnectLimits;
use manja::kite::ticker::actor::owner::{
    TaskOutcome, TerminalReason, TickerBuilder, TickerError, TickerEvent, TickerEvents,
    TickerLimits, TickerState,
};
use manja::kite::ticker::actor::subscriptions::Revision;
use manja::kite::ticker::Mode;
use tokio_tungstenite::tungstenite::Message;

use support::ws::{Handshake, Step, WsConnection, WsHarness};

fn t(v: u32) -> InstrumentToken {
    InstrumentToken::new(v)
}

fn quick(attempts: u32) -> TickerLimits {
    TickerLimits::default().with_reconnect(
        ReconnectLimits::default()
            .with_attempts(attempts)
            .unwrap()
            .with_backoff(Duration::from_millis(50), Duration::from_millis(50))
            .unwrap()
            .with_jitter_seed(1),
    )
}

fn builder(url: &str, limits: TickerLimits) -> TickerBuilder {
    TickerBuilder::new(Credentials::new("test_api_key", "test_access_token").unwrap())
        .url(url)
        .limits(limits)
}

fn accept(steps: Vec<Step>) -> WsConnection {
    WsConnection {
        handshake: Handshake::Accept,
        steps,
    }
}

// (epoch, sequence, description)
type Item = (u64, u64, String);

fn describe(item: Result<TickerEvent, TickerError>) -> Item {
    match item {
        Ok(TickerEvent::Raw(r)) => (
            r.source().connection_epoch().0,
            r.source().ingress_sequence(),
            format!("raw:{}", r.payload().len()),
        ),
        Ok(TickerEvent::Lifecycle(l)) => {
            let kind = match l.kind() {
                // Delays and times vary; keep the structure.
                LifecycleKind::Backoff { next_attempt, .. } => {
                    format!("Backoff {{ next_attempt: {next_attempt} }}")
                }
                LifecycleKind::Gap(g) => format!(
                    "Gap {{ previous_epoch: {}, last_sequence: {:?}, reason: {:?} }}",
                    g.previous_epoch.0, g.last_sequence_in_previous_epoch, g.reason
                ),
                other => format!("{other:?}"),
            };
            (
                l.source().connection_epoch().0,
                l.source().ingress_sequence(),
                kind,
            )
        }
        Ok(_) => (0, 0, "other".into()),
        Err(e) => (0, 0, format!("err:{:?}", e.reason())),
    }
}

async fn until(events: &mut TickerEvents, done: impl Fn(&str) -> bool) -> Vec<Item> {
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), events.next()).await {
            Ok(Some(item)) => {
                let item = describe(item);
                let stop = done(&item.2);
                out.push(item);
                if stop {
                    return out;
                }
            }
            Ok(None) => return out,
            Err(_) => panic!("stalled after {out:?}"),
        }
    }
}

fn names(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.2.as_str()).collect()
}

fn texts(h: &WsHarness) -> Vec<String> {
    h.messages()
        .into_iter()
        .filter_map(|m| match m {
            Message::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_lost_connection_reconnects_under_a_fresh_epoch_and_restores_before_active() {
    let h = WsHarness::start(vec![
        accept(vec![
            Step::Wait(Duration::from_millis(200)),
            Step::Send(Message::Binary(vec![0])),
            Step::Eof,
        ]),
        accept(vec![Step::Send(Message::Binary(vec![0]))]),
    ])
    .await;
    let (handle, mut events, _guard) = builder(&h.url(), quick(3)).spawn().unwrap();
    // Queued before the owner runs, so restored on the first connection.
    assert_eq!(
        handle.subscribe([t(2), t(1)], Mode::Full).await,
        Ok(Revision(1))
    );
    let items = until(&mut events, |d| d == "raw:1").await;
    let items2 = until(&mut events, |d| d == "raw:1").await;
    let all: Vec<Item> = items.into_iter().chain(items2).collect();
    assert_eq!(
        names(&all),
        [
            "ConnectAttempt { attempt: 1 }",
            "Connected",
            "CommandsSent { revision: 1 }",
            "Active { revision: 1 }",
            "raw:1",
            "Disconnected { reason: Eof }",
            "Backoff { next_attempt: 2 }",
            "ConnectAttempt { attempt: 2 }",
            "Connected",
            "Gap { previous_epoch: 1, last_sequence: Some(6), reason: Eof }",
            "CommandsSent { revision: 1 }",
            "Active { revision: 1 }",
            "raw:1",
        ]
    );
    // Epoch 1 then epoch 2, each with a gapless sequence from 0.
    let keys: Vec<(u64, u64)> = all.iter().map(|i| (i.0, i.1)).collect();
    assert_eq!(
        keys,
        [
            (1, 0),
            (1, 1),
            (1, 2),
            (1, 3),
            (1, 4),
            (1, 5),
            (1, 6),
            (2, 0),
            (2, 1),
            (2, 2),
            (2, 3),
            (2, 4),
            (2, 5),
        ]
    );
    // Both connections received the full desired map, subscribe then mode.
    let restore = [
        r#"{"a":"subscribe","v":[1,2]}"#.to_string(),
        r#"{"a":"mode","v":["full",[1,2]]}"#.to_string(),
    ];
    for _ in 0..200 {
        if texts(&h).len() >= 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(texts(&h), [restore.clone(), restore].concat());
    let status = handle.status();
    assert_eq!(
        (status.state, status.connection_epoch),
        (TickerState::Active, ConnectionEpoch(2))
    );
    assert_eq!(h.handshakes().len(), 2);
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_rejection_on_reconnect_is_terminal_with_no_further_attempt() {
    let h = WsHarness::start(vec![
        accept(vec![Step::Wait(Duration::from_millis(50)), Step::Eof]),
        WsConnection {
            handshake: Handshake::Reject(403),
            steps: vec![],
        },
        accept(vec![]),
    ])
    .await;
    let (handle, mut events, guard) = builder(&h.url(), quick(10)).spawn().unwrap();
    let items = until(&mut events, |d| d.starts_with("err:")).await;
    assert_eq!(
        &names(&items)[names(&items).len() - 3..],
        [
            "ConnectAttempt { attempt: 2 }",
            "AuthRejected { http_status: 403 }",
            "err:AuthRejected { http_status: 403 }"
        ]
    );
    assert!(events.next().await.is_none());
    assert_eq!(
        guard.join().await,
        TaskOutcome::Terminal(TerminalReason::AuthRejected { http_status: 403 })
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        h.handshakes().len(),
        2,
        "never retried with rejected credentials"
    );
    assert_eq!(handle.status().state, TickerState::AuthRejected);
}

#[tokio::test]
async fn retries_are_bounded_and_every_attempt_has_a_fresh_epoch() {
    // No scripts: every connection closes before its handshake.
    let h = WsHarness::start(vec![]).await;
    let (handle, mut events, guard) = builder(&h.url(), quick(3)).spawn().unwrap();
    let items = until(&mut events, |d| d.starts_with("err:")).await;
    let exhausted = TerminalReason::ReconnectExhausted {
        attempts: 3,
        last: Box::new(TerminalReason::ConnectFailed),
    };
    assert_eq!(
        names(&items),
        [
            "ConnectAttempt { attempt: 1 }",
            "Backoff { next_attempt: 2 }",
            "ConnectAttempt { attempt: 2 }",
            "Backoff { next_attempt: 3 }",
            "ConnectAttempt { attempt: 3 }",
            "Failed",
            &format!("err:{exhausted:?}"),
        ]
    );
    let epochs: Vec<u64> = items
        .iter()
        .filter(|i| i.2.starts_with("ConnectAttempt"))
        .map(|i| i.0)
        .collect();
    assert_eq!(epochs, [1, 2, 3]);
    assert_eq!(guard.join().await, TaskOutcome::Terminal(exhausted));
    assert_eq!(handle.status().connection_epoch, ConnectionEpoch(3));
}

// A failed restoration write is covered deterministically by the owner's
// unit tests, through a test-only fault seam: over loopback, a peer that
// drops the connection right after the handshake cannot reliably make the
// client's writes fail.

#[tokio::test]
async fn commands_during_backoff_apply_to_the_restored_map() {
    let h = WsHarness::start(vec![
        accept(vec![Step::Wait(Duration::from_millis(50)), Step::Eof]),
        accept(vec![]),
    ])
    .await;
    let limits = TickerLimits::default().with_reconnect(
        ReconnectLimits::default()
            .with_backoff(Duration::from_millis(500), Duration::from_millis(500))
            .unwrap()
            .with_jitter_seed(4),
    );
    let (handle, mut events, _guard) = builder(&h.url(), limits).spawn().unwrap();
    until(&mut events, |d| d.starts_with("Backoff")).await;
    if handle.status().state == TickerState::Backoff {
        assert_eq!(
            handle.subscribe([t(99)], Mode::Quote).await,
            Ok(Revision(1))
        );
    } else {
        // A zero jittered delay can end the backoff first; the command
        // still applies to the restored map.
        handle.subscribe([t(99)], Mode::Quote).await.unwrap();
    }
    until(&mut events, |d| d == "CommandsSent { revision: 1 }").await;
    for _ in 0..200 {
        if texts(&h).len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        texts(&h),
        [
            r#"{"a":"subscribe","v":[99]}"#,
            r#"{"a":"mode","v":["quote",[99]]}"#
        ]
    );
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_interrupts_an_outstanding_attempt_and_a_backoff() {
    // An outstanding handshake: TCP accepted, upgrade never answered.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let hold = tokio::spawn(async move {
        let _c = listener.accept().await;
        std::future::pending::<()>().await;
    });
    let (handle, mut events, guard) = builder(&url, TickerLimits::default()).spawn().unwrap();
    until(&mut events, |d| d.starts_with("ConnectAttempt")).await;
    let started = tokio::time::Instant::now();
    handle.shutdown().await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(names(&until(&mut events, |_| false).await), ["Stopped"]);
    assert_eq!(guard.join().await, TaskOutcome::Clean);
    hold.abort();

    // A long backoff.
    let h = WsHarness::start(vec![]).await;
    let limits = TickerLimits::default().with_reconnect(
        ReconnectLimits::default()
            .with_backoff(Duration::from_secs(10), Duration::from_secs(10))
            .unwrap()
            .with_jitter_seed(2),
    );
    let (handle, mut events, guard) = builder(&h.url(), limits).spawn().unwrap();
    until(&mut events, |d| d.starts_with("Backoff")).await;
    let started = tokio::time::Instant::now();
    handle.shutdown().await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(guard.join().await, TaskOutcome::Clean);
}

#[tokio::test]
async fn heartbeats_keep_the_connection_live_and_silence_is_a_liveness_timeout() {
    let heartbeat = || Step::Send(Message::Binary(vec![0]));
    let mut steps = Vec::new();
    for _ in 0..6 {
        steps.push(heartbeat());
        steps.push(Step::Wait(Duration::from_millis(500)));
    }
    // Then silence: the connection stays open but sends nothing.
    let h = WsHarness::start(vec![accept(steps), accept(vec![])]).await;
    let limits = TickerLimits::default().with_reconnect(
        ReconnectLimits::default()
            .with_liveness_timeout(Duration::from_secs(2))
            .unwrap()
            .with_backoff(Duration::from_millis(50), Duration::from_millis(50))
            .unwrap(),
    );
    let started = tokio::time::Instant::now();
    let (handle, mut events, _guard) = builder(&h.url(), limits).spawn().unwrap();
    let items = until(&mut events, |d| d.starts_with("Disconnected")).await;
    let elapsed = started.elapsed();
    // Six heartbeats over 3 s, each a raw binary observation, then the
    // timeout about 2 s after the last one.
    assert_eq!(names(&items).iter().filter(|d| **d == "raw:1").count(), 6);
    assert_eq!(
        names(&items).last().unwrap(),
        &format!(
            "Disconnected {{ reason: {:?} }}",
            DisconnectReason::LivenessTimeout
        )
    );
    assert!(elapsed >= Duration::from_millis(4400), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(8), "{elapsed:?}");
    let next = until(&mut events, |d| d.starts_with("Active")).await;
    assert!(names(&next).contains(&"ConnectAttempt { attempt: 2 }"));
    handle.shutdown().await.unwrap();
}
