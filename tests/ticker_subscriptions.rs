//! Desired subscriptions, revisions and wire order through the owner
//! (plan task S17), against loopback WebSocket servers. Request shapes follow
//! `kite-api-docs/docs/connect/v3/websocket.md:26-45`.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{FutureExt, StreamExt};
use manja::kite::connect::credentials::Credentials;
use manja::kite::protocol::InstrumentToken;
use manja::kite::ticker::actor::owner::{
    CommandError, TickerBuilder, TickerError, TickerEvent, TickerEvents, TickerLimits,
};
use manja::kite::ticker::actor::subscriptions::{Revision, SubscriptionError};
use manja::kite::ticker::Mode;
use tokio_tungstenite::tungstenite::Message;

use support::ws::{Handshake, Step, WsConnection, WsHarness};

fn t(v: u32) -> InstrumentToken {
    InstrumentToken::new(v)
}

fn builder(url: &str) -> TickerBuilder {
    TickerBuilder::new(Credentials::new("test_api_key", "test_access_token").unwrap()).url(url)
}

fn describe(item: &Result<TickerEvent, TickerError>) -> String {
    match item {
        Ok(TickerEvent::Raw(r)) => format!("raw:{}", r.payload().len()),
        Ok(TickerEvent::Lifecycle(l)) => format!("{:?}", l.kind()),
        Ok(_) => "other".into(),
        Err(e) => format!("err:{:?}", e.reason()),
    }
}

async fn take(events: &mut TickerEvents, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    while out.len() < n {
        match tokio::time::timeout(Duration::from_secs(5), events.next()).await {
            Ok(Some(item)) => out.push(describe(&item)),
            Ok(None) => break,
            Err(_) => panic!("no event within 5 s after {out:?}"),
        }
    }
    out
}

fn texts(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|m| match m {
            Message::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

async fn wait_for_texts(h: &WsHarness, n: usize) -> Vec<String> {
    for _ in 0..500 {
        let got = texts(&h.messages());
        if got.len() >= n {
            return got;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("only {:?}", texts(&h.messages()));
}

// A WebSocket server that answers the handshake only after `release` is
// notified, recording every text message.
async fn delayed_server(
    release: Arc<tokio::sync::Notify>,
) -> (String, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    let task = tokio::spawn(async move {
        release.notified().await;
        let (tcp, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let (_sink, mut source) = ws.split();
        while let Some(Ok(m)) = source.next().await {
            if let Message::Text(t) = m {
                record.lock().unwrap().push(t);
            }
        }
        std::future::pending::<()>().await;
    });
    (url, seen, task)
}

#[tokio::test]
async fn commands_before_the_connection_are_restored_subscribe_then_mode_before_active() {
    let release = Arc::new(tokio::sync::Notify::new());
    let (url, seen, server) = delayed_server(release.clone()).await;
    let (handle, mut events, _guard) = builder(&url).spawn().unwrap();
    assert_eq!(
        take(&mut events, 1).await,
        ["ConnectAttempt { attempt: 1 }"]
    );
    // Accepted while connecting: completion is acceptance.
    assert_eq!(
        handle.subscribe([t(884737), t(408065)], Mode::Full).await,
        Ok(Revision(1))
    );
    assert_eq!(
        handle
            .replace([
                (t(256265), Mode::LTP),
                (t(408065), Mode::Full),
                (t(5633), Mode::Quote)
            ])
            .await,
        Ok(Revision(2))
    );
    assert_eq!(handle.status().sent_revision, None);
    release.notify_one();
    assert_eq!(
        take(&mut events, 4).await,
        [
            "Connected",
            "Superseded { revision: 1 }",
            "CommandsSent { revision: 2 }",
            "Active { revision: 2 }",
        ]
    );
    let status = handle.status();
    assert_eq!(status.desired_revision, Revision(2));
    assert_eq!(status.sent_revision, Some(Revision(2)));
    for _ in 0..500 {
        if seen.lock().unwrap().len() >= 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        *seen.lock().unwrap(),
        [
            r#"{"a":"subscribe","v":[5633,256265,408065]}"#,
            r#"{"a":"mode","v":["ltp",[256265]]}"#,
            r#"{"a":"mode","v":["quote",[5633]]}"#,
            r#"{"a":"mode","v":["full",[408065]]}"#,
        ]
    );
    handle.shutdown().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn live_commands_reconcile_idempotently_and_refusals_change_nothing() {
    let h = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: vec![],
    }])
    .await;
    let limits = TickerLimits::default().with_max_instruments(3).unwrap();
    let (handle, mut events, _guard) = builder(&h.url()).limits(limits).spawn().unwrap();
    take(&mut events, 3).await;

    assert_eq!(
        handle.subscribe([t(2), t(1)], Mode::Quote).await,
        Ok(Revision(1))
    );
    assert_eq!(take(&mut events, 1).await, ["CommandsSent { revision: 1 }"]);
    // Identical again: same revision, no wire message, no event.
    assert_eq!(
        handle.subscribe([t(1), t(2)], Mode::Quote).await,
        Ok(Revision(1))
    );
    assert_eq!(handle.unsubscribe([t(9)]).await, Ok(Revision(1)));
    // Refusals.
    assert_eq!(
        handle.set_mode([t(1), t(7)], Mode::Full).await,
        Err(CommandError::Invalid(SubscriptionError::NotDesired {
            token: t(7)
        }))
    );
    assert_eq!(
        handle
            .replace([(t(1), Mode::Full), (t(1), Mode::LTP)])
            .await,
        Err(CommandError::Invalid(SubscriptionError::ConflictingModes {
            token: t(1)
        }))
    );
    assert_eq!(
        handle.subscribe([t(3), t(4)], Mode::LTP).await,
        Err(CommandError::Invalid(SubscriptionError::CapacityExceeded {
            requested: 4,
            max: 3
        }))
    );
    assert_eq!(handle.status().desired_revision, Revision(1));
    // A mode change and an unsubscribe.
    assert_eq!(handle.set_mode([t(2)], Mode::Full).await, Ok(Revision(2)));
    assert_eq!(take(&mut events, 1).await, ["CommandsSent { revision: 2 }"]);
    assert_eq!(handle.unsubscribe([t(1)]).await, Ok(Revision(3)));
    assert_eq!(take(&mut events, 1).await, ["CommandsSent { revision: 3 }"]);
    assert_eq!(
        wait_for_texts(&h, 4).await,
        [
            r#"{"a":"subscribe","v":[1,2]}"#,
            r#"{"a":"mode","v":["quote",[1,2]]}"#,
            r#"{"a":"mode","v":["full",[2]]}"#,
            r#"{"a":"unsubscribe","v":[1]}"#,
        ]
    );
    handle.shutdown().await.unwrap();
}

fn flood(n: usize) -> Vec<Step> {
    (0..n)
        .map(|_| Step::Send(Message::Binary(vec![0; 8])))
        .collect()
}

#[tokio::test]
async fn a_full_mailbox_is_an_immediate_error_and_dropped_commands_are_not_rolled_back() {
    // The consumer does not read, so the owner blocks on delivery and
    // stops taking commands.
    let h = WsHarness::start(vec![WsConnection {
        handshake: Handshake::Accept,
        steps: flood(100),
    }])
    .await;
    let limits = TickerLimits::default()
        .with_queue_messages(16)
        .unwrap()
        .with_command_mailbox(1)
        .unwrap()
        .with_max_queue_age(Duration::from_secs(60))
        .unwrap()
        .with_delivery_wait(Duration::from_secs(30))
        .unwrap();
    let (handle, mut events, _guard) = builder(&h.url()).limits(limits).spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Submitted on the first poll, then abandoned.
    assert!(handle
        .subscribe([t(42)], Mode::LTP)
        .now_or_never()
        .is_none());
    // The mailbox holds that one command: the next is refused at once.
    let refused = tokio::time::timeout(
        Duration::from_millis(100),
        handle.subscribe([t(43)], Mode::LTP),
    )
    .await
    .expect("no indefinite wait");
    assert_eq!(refused, Err(CommandError::MailboxFull));
    // Unblock the owner: the abandoned command is applied and sent.
    let mut seen = Vec::new();
    while !seen.iter().any(|d: &String| d.starts_with("CommandsSent")) {
        seen.extend(take(&mut events, 1).await);
    }
    assert!(seen.contains(&"CommandsSent { revision: 1 }".to_string()));
    assert_eq!(handle.status().desired_revision, Revision(1));
    let wire = wait_for_texts(&h, 2).await;
    assert_eq!(
        wire,
        [
            r#"{"a":"subscribe","v":[42]}"#,
            r#"{"a":"mode","v":["ltp",[42]]}"#
        ]
    );
    // A terminated ticker refuses commands with its reason.
    drop(events);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(matches!(
        handle.subscribe([t(44)], Mode::LTP).await,
        Err(CommandError::Terminated(_))
    ));
}
