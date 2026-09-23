//! Bounded delivery, cancellation and teardown, against the
//! loopback WebSocket harness. Payloads are opaque bytes; faults are
//! labelled supplements.

mod support;

use std::time::Duration;

use futures_util::{FutureExt, StreamExt};
use manja::kite::connect::credentials::Credentials;
use manja::kite::ticker::actor::lifecycle::ReconnectLimits;
use manja::kite::ticker::actor::owner::{
    TaskOutcome, TerminalReason, TickerBuilder, TickerEvent, TickerEvents, TickerLimits,
};
use tokio_tungstenite::tungstenite::Message;

use support::ws::{Handshake, Step, WsConnection, WsHarness};

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

fn numbered(n: usize) -> Vec<Step> {
    (0..n)
        .map(|i| Step::Send(Message::Binary((i as u32).to_be_bytes().to_vec())))
        .collect()
}

// Raw payload numbers and the terminal error, draining to the end.
async fn drain(events: &mut TickerEvents) -> (Vec<u32>, Option<TerminalReason>) {
    let mut raw = Vec::new();
    let mut error = None;
    while let Some(item) = tokio::time::timeout(Duration::from_secs(10), events.next())
        .await
        .expect("the stream ends")
    {
        match item {
            Ok(TickerEvent::Raw(r)) => raw.push(u32::from_be_bytes(
                r.payload().as_bytes().try_into().unwrap(),
            )),
            Ok(_) => {}
            Err(e) => {
                assert!(error.is_none(), "one terminal error");
                error = Some(e.reason().clone());
            }
        }
    }
    (raw, error)
}

#[tokio::test]
async fn a_consumer_that_falls_behind_fails_delivery_explicitly_and_loses_nothing_read() {
    let h = WsHarness::start(vec![accept(numbered(5))]).await;
    let limits = TickerLimits::default()
        .with_delivery_wait(Duration::from_millis(100))
        .unwrap()
        .with_max_queue_age(Duration::from_millis(200))
        .unwrap();
    let (_handle, mut events, guard) = builder(&h.url(), limits).spawn().unwrap();
    // The consumer does not read: the oldest queued event ages past
    // B-TK-07.
    let started = tokio::time::Instant::now();
    assert_eq!(
        guard.join().await,
        TaskOutcome::Terminal(TerminalReason::DeliveryOverload)
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    let (raw, error) = drain(&mut events).await;
    assert_eq!(raw, [0, 1, 2, 3, 4], "everything read was delivered");
    assert_eq!(error, Some(TerminalReason::DeliveryOverload));
}

#[tokio::test]
async fn cancelled_polls_never_lose_or_repeat_an_observation() {
    let h = WsHarness::start(vec![accept(numbered(300))]).await;
    let (handle, mut events, _guard) = builder(&h.url(), TickerLimits::default()).spawn().unwrap();
    let mut raw = Vec::new();
    let mut cancelled = 0;
    let mut sequences = Vec::new();
    while raw.len() < 300 {
        // One poll, then the future is dropped: whenever nothing is ready,
        // the poll is cancelled.
        match events.next().now_or_never() {
            None => {
                cancelled += 1;
                tokio::task::yield_now().await;
            }
            Some(Some(Ok(TickerEvent::Raw(r)))) => {
                sequences.push(r.source().ingress_sequence());
                raw.push(u32::from_be_bytes(
                    r.payload().as_bytes().try_into().unwrap(),
                ));
            }
            Some(Some(Ok(_))) => {}
            Some(other) => panic!("{other:?}"),
        }
    }
    assert!(cancelled > 0, "some polls were cancelled");
    assert_eq!(raw, (0..300).collect::<Vec<u32>>());
    assert!(sequences.windows(2).all(|w| w[1] == w[0] + 1));
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn dropping_the_receiver_reports_what_it_still_held() {
    let h = WsHarness::start(vec![accept(numbered(10))]).await;
    let (_handle, events, guard) = builder(&h.url(), TickerLimits::default()).spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(events);
    match guard.join().await {
        TaskOutcome::Terminal(TerminalReason::ReceiverDropped { undelivered }) => {
            // 3 lifecycle events and 10 observations were queued.
            assert_eq!(undelivered, 13);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_unanswered_close_is_a_failed_close_not_a_clean_end() {
    // A peer that completes the handshake and then never reads, so it never
    // answers the close frame.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let _ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        std::future::pending::<()>().await;
    });
    let limits = TickerLimits::default()
        .with_shutdown_deadline(Duration::from_millis(200))
        .unwrap();
    let (handle, mut events, guard) = builder(&url, limits).spawn().unwrap();
    let mut seen = 0;
    while seen < 3 {
        events.next().await.unwrap().unwrap();
        seen += 1;
    }
    let consumer = tokio::spawn(async move { drain(&mut events).await });
    let started = tokio::time::Instant::now();
    assert_eq!(
        handle.shutdown().await.unwrap_err().reason(),
        &TerminalReason::CloseFailed
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        guard.join().await,
        TaskOutcome::Terminal(TerminalReason::CloseFailed)
    );
    assert_eq!(consumer.await.unwrap().1, Some(TerminalReason::CloseFailed));
    server.abort();
}

#[tokio::test]
async fn many_tickers_end_within_bounds_and_leave_no_task_behind() {
    let metrics = tokio::runtime::Handle::current().metrics();
    let baseline = metrics.num_alive_tasks();
    let n = 24;
    // Tickers take connections in the order they connect, so every script
    // is the same.
    let connections = (0..n).map(|_| accept(numbered(20))).collect();
    let h = WsHarness::start(connections).await;
    let limits = TickerLimits::default()
        .with_reconnect(ReconnectLimits::default().with_attempts(1).unwrap());
    let mut guards = Vec::new();
    let mut ends = Vec::new();
    for i in 0..n {
        let (handle, mut events, guard) = builder(&h.url(), limits.clone()).spawn().unwrap();
        guards.push(guard);
        ends.push(tokio::spawn(async move {
            match i % 4 {
                // Clean shutdown with a draining consumer.
                0 => {
                    let consumer = tokio::spawn(async move { drain(&mut events).await });
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    handle.shutdown().await.unwrap();
                    consumer.await.unwrap();
                }
                // The consumer goes away.
                1 => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    drop(events);
                    drop(handle);
                }
                // Every handle goes away.
                2 => {
                    drop(handle);
                    drain(&mut events).await;
                }
                // Shutdown with nobody reading: the queue has room, so it
                // is still clean.
                _ => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    handle.shutdown().await.unwrap();
                    drop(events);
                }
            }
        }));
    }
    let started = tokio::time::Instant::now();
    for end in ends {
        end.await.unwrap();
    }
    let mut outcomes = Vec::new();
    for guard in guards {
        outcomes.push(
            tokio::time::timeout(Duration::from_secs(10), guard.join())
                .await
                .expect("bounded termination"),
        );
    }
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == TaskOutcome::Clean)
            .count(),
        n / 2
    );
    drop(h);
    // Owner tasks, harness tasks and consumers are all gone.
    for _ in 0..200 {
        if metrics.num_alive_tasks() <= baseline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        metrics.num_alive_tasks() <= baseline,
        "{} alive, baseline {baseline}",
        metrics.num_alive_tasks()
    );
}
