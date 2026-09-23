//! The supervised single-owner raw ticker (plan task S16), against the
//! loopback WebSocket harness. No HTTP session API, decoder or storage is
//! used.
//!
//! Binary payloads are the vendored protocol fixtures
//! (`tests/fixtures/ticker/protocol/`), delivered unchanged; the text
//! message and the faults are labelled supplements.

mod support;

use std::time::Duration;

use futures_util::StreamExt;
use manja::kite::connect::credentials::Credentials;
use manja::kite::envelope::{ConnectionEpoch, DisconnectReason, PayloadKind};
use manja::kite::ticker::actor::lifecycle::ReconnectLimits;
use manja::kite::ticker::actor::owner::{
    TaskOutcome, TerminalReason, TickerBuilder, TickerError, TickerEvent, TickerEvents,
    TickerLimits, TickerSpawnError, TickerState,
};
use tokio_tungstenite::tungstenite::Message;

use support::capture::read_ticker_fixture;
use support::ws::{Handshake, Step, WsConnection, WsHarness};

const TOKEN: &str = "SENTINELtoken0001";

fn builder(url: &str) -> TickerBuilder {
    TickerBuilder::new(Credentials::new("test_api_key", TOKEN).unwrap()).url(url)
}

fn accept(steps: Vec<Step>) -> WsConnection {
    WsConnection {
        handshake: Handshake::Accept,
        steps,
    }
}

fn binary(name: &str) -> Vec<u8> {
    read_ticker_fixture(name)
}

// Supplemental: a postback-shaped text message.
const TEXT: &str = r#"{"type":"message","data":"hello"}"#;

// A readable description of each item, and the terminal error if any.
fn describe(item: &Result<TickerEvent, TickerError>) -> String {
    match item {
        Ok(TickerEvent::Raw(r)) => format!("raw:{:?}:{}", r.kind(), r.payload().len()),
        Ok(TickerEvent::Lifecycle(l)) => format!("{:?}", l.kind()),
        Ok(_) => "other".into(),
        Err(e) => format!("err:{:?}", e.reason()),
    }
}

async fn take(events: &mut TickerEvents, n: usize) -> Vec<Result<TickerEvent, TickerError>> {
    let mut out = Vec::new();
    while out.len() < n {
        match tokio::time::timeout(Duration::from_secs(5), events.next()).await {
            Ok(Some(item)) => out.push(item),
            Ok(None) => break,
            Err(_) => panic!("no event within 5 s after {out:?}"),
        }
    }
    out
}

async fn drain(events: &mut TickerEvents) -> Vec<Result<TickerEvent, TickerError>> {
    take(events, usize::MAX).await
}

fn key(item: &Result<TickerEvent, TickerError>) -> (u64, u64) {
    let source = match item {
        Ok(TickerEvent::Raw(r)) => r.source(),
        Ok(TickerEvent::Lifecycle(l)) => l.source(),
        _ => panic!("no source"),
    };
    (source.connection_epoch().0, source.ingress_sequence())
}

#[tokio::test]
async fn raw_messages_arrive_in_source_order_before_any_decoding_and_shutdown_is_clean_eof() {
    let heartbeat = binary("protocol/heartbeat.bin");
    let full = binary("protocol/single_full.bin");
    let h = WsHarness::start(vec![accept(vec![
        Step::Send(Message::Binary(heartbeat.clone())),
        Step::Send(Message::Text(TEXT.into())),
        Step::Send(Message::Ping(vec![1])),
        Step::Send(Message::Binary(full.clone())),
    ])])
    .await;
    let (handle, mut events, guard) = builder(&h.url()).spawn().unwrap();
    // Consumed on another task: the stream is Send + 'static.
    let consumer = tokio::spawn(async move {
        let first = take(&mut events, 6).await;
        (first, events)
    });
    let (first, mut events) = consumer.await.unwrap();
    let described: Vec<String> = first.iter().map(describe).collect();
    assert_eq!(
        described,
        [
            "ConnectAttempt { attempt: 1 }".to_string(),
            "Connected".into(),
            "Active { revision: 0 }".into(),
            format!("raw:Binary:{}", heartbeat.len()),
            format!("raw:Text:{}", TEXT.len()),
            format!("raw:Binary:{}", full.len()),
        ]
    );
    // The payloads are the bytes sent, unchanged, and the ping is not a
    // message.
    let raws: Vec<_> = first
        .iter()
        .filter_map(|i| match i {
            Ok(TickerEvent::Raw(r)) => Some(r.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(raws[0].payload().as_bytes(), heartbeat);
    assert_eq!(raws[1].text(), Some(TEXT));
    assert_eq!(raws[1].kind(), PayloadKind::Text);
    assert_eq!(raws[2].payload().as_bytes(), full);
    // One epoch, gapless ingress sequence in delivery order.
    let keys: Vec<_> = first.iter().map(key).collect();
    assert_eq!(keys, (0..6).map(|s| (1, s)).collect::<Vec<_>>());
    let status = handle.status();
    assert_eq!(status.state, TickerState::Active);
    assert_eq!(status.connection_epoch, ConnectionEpoch(1));
    assert_eq!(status.terminal, None);

    // The handshake carried the supplied credentials in the documented
    // query (`websocket.md:20`), and nothing else authenticates.
    let [hs] = h.handshakes().try_into().unwrap();
    assert_eq!(
        hs.target,
        format!("/?api_key=test_api_key&access_token={TOKEN}")
    );

    handle.shutdown().await.unwrap();
    let rest: Vec<String> = drain(&mut events).await.iter().map(describe).collect();
    assert_eq!(rest, ["Disconnected { reason: Shutdown }", "Stopped"]);
    assert!(events.next().await.is_none(), "EOF stays EOF");
    assert_eq!(guard.join().await, TaskOutcome::Clean);
    let status = handle.status();
    assert_eq!(status.state, TickerState::Stopped);
    assert_eq!(status.terminal, Some(TerminalReason::Shutdown));
    // The client closed the connection with a close frame.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.messages().iter().any(|m| matches!(m, Message::Close(_))));
    // No debug rendering carries the token.
    let rendered = format!("{handle:?} {events:?} {status:?}");
    assert!(!rendered.contains(TOKEN), "{rendered}");
}

fn one_attempt() -> ReconnectLimits {
    ReconnectLimits::default().with_attempts(1).unwrap()
}

fn exhausted(last: TerminalReason) -> TerminalReason {
    TerminalReason::ReconnectExhausted {
        attempts: 1,
        last: Box::new(last),
    }
}

async fn terminal_run(
    connections: Vec<WsConnection>,
    limits: TickerLimits,
) -> (Vec<String>, TaskOutcome, TickerState, u64, usize) {
    let h = WsHarness::start(connections).await;
    // One attempt: these cases are about how a single connection ends;
    // reconnecting is covered by ticker_lifecycle.
    let limits = limits.with_reconnect(one_attempt());
    let (handle, mut events, guard) = builder(&h.url()).limits(limits).spawn().unwrap();
    let items: Vec<String> = drain(&mut events).await.iter().map(describe).collect();
    // Reported once: nothing after the terminal error.
    assert!(events.next().await.is_none());
    assert!(events.next().await.is_none());
    let outcome = guard.join().await;
    let status = handle.status();
    assert!(status.terminal.is_some(), "the terminal reason is sticky");
    (
        items,
        outcome,
        status.state,
        status.connection_epoch.0,
        h.handshakes().len(),
    )
}

#[tokio::test]
async fn a_rejected_handshake_is_terminal_once_with_one_attempt() {
    for (status, expected) in [
        (403, TerminalReason::AuthRejected { http_status: 403 }),
        (401, TerminalReason::AuthRejected { http_status: 401 }),
    ] {
        let (items, outcome, state, epoch, handshakes) = terminal_run(
            vec![
                WsConnection {
                    handshake: Handshake::Reject(status),
                    steps: vec![],
                },
                accept(vec![]),
            ],
            TickerLimits::default(),
        )
        .await;
        assert_eq!(
            items,
            [
                "ConnectAttempt { attempt: 1 }".to_string(),
                format!("AuthRejected {{ http_status: {status} }}"),
                format!("err:{expected:?}"),
            ]
        );
        assert_eq!(outcome, TaskOutcome::Terminal(expected));
        assert_eq!(
            (state, epoch, handshakes),
            (TickerState::AuthRejected, 1, 1)
        );
    }
    let (items, outcome, state, _, _) = terminal_run(
        vec![WsConnection {
            handshake: Handshake::Reject(500),
            steps: vec![],
        }],
        TickerLimits::default(),
    )
    .await;
    let reason = exhausted(TerminalReason::HandshakeRejected { http_status: 500 });
    assert_eq!(
        items,
        [
            "ConnectAttempt { attempt: 1 }".to_string(),
            "Failed".into(),
            format!("err:{reason:?}")
        ]
    );
    assert_eq!(outcome, TaskOutcome::Terminal(reason));
    assert_eq!(state, TickerState::Failed);
}

#[tokio::test]
async fn connection_ends_are_explicit_terminal_states() {
    for (end, reason) in [
        (Step::Close, DisconnectReason::RemoteClose),
        (Step::Eof, DisconnectReason::Eof),
    ] {
        let (items, outcome, state, epoch, _) = terminal_run(
            vec![accept(vec![Step::Send(Message::Binary(vec![0])), end])],
            TickerLimits::default(),
        )
        .await;
        let terminal = exhausted(TerminalReason::Disconnected(reason));
        assert_eq!(
            items,
            [
                "ConnectAttempt { attempt: 1 }".to_string(),
                "Connected".into(),
                "Active { revision: 0 }".into(),
                "raw:Binary:1".into(),
                format!("Disconnected {{ reason: {reason:?} }}"),
                "Failed".into(),
                format!("err:{terminal:?}"),
            ]
        );
        assert_eq!(outcome, TaskOutcome::Terminal(terminal));
        assert_eq!((state, epoch), (TickerState::Failed, 1));
    }
}

#[tokio::test]
async fn a_message_over_the_payload_bound_fails_the_connection() {
    let limits = TickerLimits::default().with_max_payload(64 << 10).unwrap();
    let (items, outcome, _, _, _) = terminal_run(
        vec![accept(vec![Step::Send(Message::Binary(vec![
            0;
            (64 << 10)
                + 1
        ]))])],
        limits,
    )
    .await;
    assert!(
        items.contains(&"Disconnected { reason: ProtocolError }".to_string()),
        "{items:?}"
    );
    assert!(!items.iter().any(|i| i.starts_with("raw:")));
    assert_eq!(
        outcome,
        TaskOutcome::Terminal(exhausted(TerminalReason::Disconnected(
            DisconnectReason::ProtocolError
        )))
    );
}

#[tokio::test]
async fn a_server_lost_mid_handshake_or_stalled_is_an_explicit_failure_with_a_fresh_epoch() {
    // No script: the harness closes the connection before the handshake.
    let (items, outcome, state, epoch, _) = terminal_run(vec![], TickerLimits::default()).await;
    let reason = exhausted(TerminalReason::ConnectFailed);
    assert_eq!(
        items,
        [
            "ConnectAttempt { attempt: 1 }".to_string(),
            "Failed".into(),
            format!("err:{reason:?}")
        ]
    );
    assert_eq!(outcome, TaskOutcome::Terminal(reason));
    assert_eq!((state, epoch), (TickerState::Failed, 1));

    // A peer that accepts TCP and never answers the upgrade.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let hold = tokio::spawn(async move {
        let _conn = listener.accept().await;
        std::future::pending::<()>().await;
    });
    let limits = TickerLimits::default()
        .with_handshake_timeout(Duration::from_secs(1))
        .unwrap()
        .with_reconnect(one_attempt());
    let started = tokio::time::Instant::now();
    let (handle, mut events, guard) = builder(&url).limits(limits).spawn().unwrap();
    let items: Vec<String> = drain(&mut events).await.iter().map(describe).collect();
    let reason = exhausted(TerminalReason::HandshakeTimeout);
    assert_eq!(items.last().unwrap(), &format!("err:{reason:?}"));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(guard.join().await, TaskOutcome::Terminal(reason));
    assert_eq!(handle.status().connection_epoch, ConnectionEpoch(1));
    hold.abort();
}

fn flood(n: usize) -> Vec<Step> {
    (0..n)
        .map(|i| Step::Send(Message::Binary(vec![i as u8; 8])))
        .collect()
}

#[tokio::test]
async fn a_full_queue_leaves_status_and_shutdown_responsive() {
    let h = WsHarness::start(vec![accept(flood(100))]).await;
    let limits = TickerLimits::default()
        .with_queue_messages(16)
        .unwrap()
        .with_max_queue_age(Duration::from_secs(60))
        .unwrap()
        .with_delivery_wait(Duration::from_secs(30))
        .unwrap();
    let (handle, mut events, guard) = builder(&h.url()).limits(limits).spawn().unwrap();
    // Let the owner fill the queue and block on delivery.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let before = handle.status();
    assert_eq!(before.state, TickerState::Active);
    for _ in 0..100 {
        let _ = handle.status();
    }
    // Shutdown is accepted with the queue full; it finishes once the
    // consumer makes room, and every accepted observation is kept.
    let stopping = tokio::spawn({
        let handle = handle.clone();
        async move { handle.shutdown().await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(handle.status().state, TickerState::Active);
    let items = drain(&mut events).await;
    stopping.await.unwrap().unwrap();
    assert_eq!(guard.join().await, TaskOutcome::Clean);
    let described: Vec<String> = items.iter().map(describe).collect();
    let raw = described.iter().filter(|d| d.starts_with("raw:")).count();
    // 16 slots hold the 3 initial lifecycle events and 13 observations;
    // the owner held one more while it waited for room.
    assert_eq!(raw, 14, "{described:?}");
    assert_eq!(
        &described[described.len() - 2..],
        ["Disconnected { reason: Shutdown }", "Stopped"]
    );
    let keys: Vec<_> = items.iter().map(key).collect();
    assert_eq!(
        keys,
        (0..keys.len() as u64).map(|s| (1, s)).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn shutdown_that_cannot_deliver_expires_within_its_deadline() {
    let h = WsHarness::start(vec![accept(flood(100))]).await;
    let limits = TickerLimits::default()
        .with_queue_messages(16)
        .unwrap()
        .with_max_queue_age(Duration::from_secs(60))
        .unwrap()
        .with_delivery_wait(Duration::from_secs(30))
        .unwrap()
        .with_shutdown_deadline(Duration::from_millis(200))
        .unwrap();
    let (handle, mut events, guard) = builder(&h.url()).limits(limits).spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let started = tokio::time::Instant::now();
    let err = handle.shutdown().await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        matches!(err.reason(), TerminalReason::ShutdownDeadlineExpired { undelivered } if *undelivered >= 1),
        "{err:?}"
    );
    assert!(matches!(
        guard.join().await,
        TaskOutcome::DeadlineExpired { .. }
    ));
    // Everything queued is still delivered, then the error, once.
    let items: Vec<String> = drain(&mut events).await.iter().map(describe).collect();
    assert_eq!(items.iter().filter(|d| d.starts_with("raw:")).count(), 13);
    assert!(items
        .last()
        .unwrap()
        .starts_with("err:ShutdownDeadlineExpired"));
}

#[tokio::test]
async fn an_unread_queue_fails_with_delivery_overload_not_a_silent_drop() {
    let h = WsHarness::start(vec![accept(flood(100))]).await;
    let limits = TickerLimits::default()
        .with_queue_messages(16)
        .unwrap()
        .with_delivery_wait(Duration::from_millis(100))
        .unwrap();
    let (_handle, mut events, guard) = builder(&h.url()).limits(limits).spawn().unwrap();
    assert_eq!(
        guard.join().await,
        TaskOutcome::Terminal(TerminalReason::DeliveryOverload)
    );
    let items: Vec<String> = drain(&mut events).await.iter().map(describe).collect();
    assert_eq!(items.last().unwrap(), "err:DeliveryOverload");
}

#[tokio::test]
async fn dropping_the_receiver_or_every_handle_terminates_the_owner() {
    let h = WsHarness::start(vec![accept(vec![]), accept(vec![])]).await;
    let (handle, mut events, guard) = builder(&h.url()).spawn().unwrap();
    take(&mut events, 3).await;
    drop(events);
    // Nothing was queued when the receiver went away.
    assert_eq!(
        guard.join().await,
        TaskOutcome::Terminal(TerminalReason::ReceiverDropped { undelivered: 0 })
    );
    assert_eq!(handle.status().state, TickerState::Failed);

    let (handle, mut events, guard) = builder(&h.url()).spawn().unwrap();
    take(&mut events, 3).await;
    let clone = handle.clone();
    drop(handle);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        clone.status().state,
        TickerState::Active,
        "a clone keeps it alive"
    );
    drop(clone);
    assert_eq!(
        guard.join().await,
        TaskOutcome::Terminal(TerminalReason::HandlesDropped)
    );
    let items: Vec<String> = drain(&mut events).await.iter().map(describe).collect();
    assert_eq!(items, ["Failed", "err:HandlesDropped"]);
}

#[tokio::test]
async fn a_dropped_guard_leaves_the_outcome_visible_and_the_owner_running() {
    let h = WsHarness::start(vec![accept(vec![Step::Send(Message::Binary(vec![0]))])]).await;
    let (handle, mut events, guard) = builder(&h.url()).spawn().unwrap();
    drop(guard);
    let items: Vec<String> = take(&mut events, 4).await.iter().map(describe).collect();
    assert_eq!(items.last().unwrap(), "raw:Binary:1");
    handle.shutdown().await.unwrap();
    assert_eq!(handle.status().terminal, Some(TerminalReason::Shutdown));
}

#[tokio::test]
async fn invalid_urls_are_rejected_before_spawning() {
    for url in ["http://127.0.0.1:1", "ws://127.0.0.1:1/?x=1", "ws://h#f"] {
        assert_eq!(
            builder(url).spawn().unwrap_err(),
            TickerSpawnError::InvalidUrl,
            "{url}"
        );
    }
}

// The legacy client is deprecated; this checks it still behaves as before.
#[allow(deprecated)]
#[tokio::test]
async fn the_legacy_stream_still_connects_and_yields_raw_messages() {
    use manja::kite::ticker::{StreamState, WebSocketClient};
    let h = WsHarness::start(vec![accept(vec![Step::Send(Message::Binary(vec![0]))])]).await;
    let state = StreamState::from_parts(h.url(), "test_api_key".into(), TOKEN.into());
    let mut legacy = WebSocketClient::connect(state).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), legacy.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first, Message::Binary(vec![0]));
    assert_eq!(h.handshakes().len(), 1);
}
