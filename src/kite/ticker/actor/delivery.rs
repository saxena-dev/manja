//! Bounded delivery, cancellation and teardown.
//!
//! # Bounds
//!
//! The primary queue is bounded on five dimensions, and each has an
//! explicit outcome when reached:
//!
//! | Bound | When reached |
//! |---|---|
//! | messages (`B-TK-05`) | the owner stops reading the socket and waits for room |
//! | retained bytes (`B-TK-06`) | the same; an item is charged for the whole backing allocation it keeps alive plus its own size, so a small slice of a large buffer is charged for the buffer |
//! | payload size (`B-TK-10`) | the message fails the connection as a protocol error; it is never truncated |
//! | oldest queued age (`B-TK-07`) | the consumer is not keeping up: delivery fails with [`TerminalReason::DeliveryOverload`] and the connection stops |
//! | delivery wait (`B-TK-08`) | the same, when no room appears within the wait |
//!
//! Nothing is dropped silently: an observation the owner has read is either
//! delivered, or the ticker ends with a terminal reason that says so. There
//! is no best-effort secondary stream in this module; the primary stream is
//! the only authoritative delivery.
//!
//! The command mailbox is bounded by `B-TK-09`, and at most one socket write
//! is ever in progress (`B-TK-13`).
//!
//! # Cancellation
//!
//! | Boundary | What cancelling it means |
//! |---|---|
//! | handshake | nothing was accepted; shutdown ends the attempt at once |
//! | backoff | nothing is pending; shutdown ends the wait at once |
//! | receive | reading a message is cancel-safe: a partly read message stays buffered in the socket, and no message is accepted until it is complete |
//! | delivery | dropping a pending `next()` loses nothing; an event leaves the queue only when it is returned |
//! | send | a subscription write interrupted by shutdown is neither retried nor reported as sent: the ticker ends with [`TerminalReason::SendInterrupted`], because the broker may have received part or all of it. A write that stalls past `B-TK-04` is a liveness loss; the next connection is restored from the desired map, never by repeating the interrupted frame |
//!
//! # Teardown
//!
//! Dropping the primary receiver ends the owner with
//! [`TerminalReason::ReceiverDropped`] and the number of events it still
//! held; dropping every handle ends it with
//! [`TerminalReason::HandlesDropped`]. A panic, a failed close, undelivered
//! events at a shutdown deadline and an interrupted send are distinct
//! terminal reasons, none of them a clean end. Shutdown claims nothing about
//! any telemetry recorder: flushing or exporting is the host's concern.
//!
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::time::Instant;

#[cfg(doc)]
use crate::kite::ticker::actor::owner::TerminalReason;
use crate::kite::ticker::actor::owner::TickerEvent;
use crate::kite::ticker::actor::status::QueueGauges;

/// The queue charge of one event: its own size plus the whole backing
/// allocation of its payload.
pub(crate) fn charge(event: &TickerEvent) -> usize {
    let payload = match event {
        TickerEvent::Raw(r) => r.payload().retained_bytes(),
        TickerEvent::Lifecycle(_) => 0,
    };
    std::mem::size_of::<TickerEvent>() + payload
}

pub(crate) struct Queued {
    pub(crate) event: TickerEvent,
    charge: usize,
    _charge: OwnedSemaphorePermit,
}

// Queue state shared by both halves: enqueue times and charges, oldest
// first, and the queue's gauge contributions when a recorder is attached.
#[derive(Default)]
struct State {
    entries: VecDeque<(Instant, usize)>,
    retained: usize,
    gauges: Option<QueueGauges>,
}

impl State {
    fn publish(&self) {
        if let Some(g) = &self.gauges {
            g.set(
                self.entries.len(),
                self.retained,
                self.entries.front().map(|e| e.0),
            );
        }
    }
}

type Shared = Arc<Mutex<State>>;

fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, State> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// The owner's half of the primary queue.
pub(crate) struct Sender {
    tx: mpsc::Sender<Queued>,
    bytes: Arc<Semaphore>,
    byte_limit: usize,
    shared: Shared,
}

/// The consumer's half.
pub(crate) struct Receiver {
    rx: mpsc::Receiver<Queued>,
    shared: Shared,
}

/// Live measures of the primary queue, readable from any handle.
#[derive(Clone)]
pub(crate) struct Stats(Shared);

impl Stats {
    /// Events queued, bytes charged and the oldest enqueue time.
    pub(crate) fn read(&self) -> (usize, usize, Option<Instant>) {
        let s = lock(&self.0);
        (s.entries.len(), s.retained, s.entries.front().map(|e| e.0))
    }
}

/// A bounded queue of `messages` events and `bytes` charged bytes.
pub(crate) fn queue(
    messages: usize,
    bytes: usize,
    gauges: Option<QueueGauges>,
) -> (Sender, Receiver) {
    let (tx, rx) = mpsc::channel(messages);
    let shared = Shared::new(Mutex::new(State {
        gauges,
        ..State::default()
    }));
    (
        Sender {
            tx,
            bytes: Arc::new(Semaphore::new(bytes)),
            byte_limit: bytes,
            shared: shared.clone(),
        },
        Receiver { rx, shared },
    )
}

/// Room for one event, reserved on both dimensions.
pub(crate) struct Room {
    slot: mpsc::OwnedPermit<Queued>,
    charge: OwnedSemaphorePermit,
    shared: Shared,
}

impl Room {
    /// Queue `event` in the reserved room.
    pub(crate) fn send(self, event: TickerEvent) {
        let charge = self.charge.num_permits();
        let mut state = lock(&self.shared);
        state.entries.push_back((Instant::now(), charge));
        state.retained += charge;
        state.publish();
        self.slot.send(Queued {
            event,
            charge,
            _charge: self.charge,
        });
    }
}

/// The receiver is gone.
#[derive(Debug)]
pub(crate) struct Closed;

impl Sender {
    /// Wait for room for an event charged `charge` bytes. Cancel-safe:
    /// dropping the future releases anything reserved.
    pub(crate) fn reserve(
        &self,
        charge: usize,
    ) -> impl std::future::Future<Output = Result<Room, Closed>> + Send + 'static + use<> {
        let (tx, bytes, shared) = (self.tx.clone(), self.bytes.clone(), self.shared.clone());
        let charge = charge.min(self.byte_limit) as u32;
        async move {
            let slot = tx.reserve_owned().await.map_err(|_| Closed)?;
            let charge = bytes.acquire_many_owned(charge).await.map_err(|_| Closed)?;
            Ok(Room {
                slot,
                charge,
                shared,
            })
        }
    }

    /// Resolves when the receiver has been dropped.
    pub(crate) async fn closed(&self) {
        self.tx.closed().await
    }

    /// When the oldest queued event was queued, if any.
    pub(crate) fn oldest(&self) -> Option<Instant> {
        lock(&self.shared).entries.front().map(|e| e.0)
    }

    /// Events queued and not yet taken.
    pub(crate) fn len(&self) -> usize {
        lock(&self.shared).entries.len()
    }

    /// Bytes charged to queued events.
    #[cfg(test)]
    pub(crate) fn retained(&self) -> usize {
        lock(&self.shared).retained
    }

    /// Live measures for status snapshots.
    pub(crate) fn stats(&self) -> Stats {
        Stats(self.shared.clone())
    }

    /// A second owner-side handle, held by the supervisor so the queue does
    /// not end before the terminal reason is recorded.
    pub(crate) fn keep_open(&self) -> mpsc::Sender<Queued> {
        self.tx.clone()
    }
}

impl Receiver {
    /// Poll for the next event. Cancel-safe.
    pub(crate) fn poll_recv(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<TickerEvent>> {
        self.rx.poll_recv(cx).map(|q| {
            q.map(|q| {
                let mut state = lock(&self.shared);
                state.entries.pop_front();
                state.retained -= q.charge;
                state.publish();
                drop(state);
                // The byte charge is released as ownership passes on.
                q.event
            })
        })
    }

    /// Events queued.
    pub(crate) fn len(&self) -> usize {
        self.rx.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kite::envelope::{
        MonotonicElapsed, PayloadKind, RawObservation, ReceiveTime, SourceIdentity, SourceSequencer,
    };

    fn raw(payload: crate::kite::envelope::Payload) -> TickerEvent {
        let mut s = SourceSequencer::new(SourceIdentity::generate());
        s.begin_epoch();
        TickerEvent::Raw(
            RawObservation::new(
                s.next_key(),
                PayloadKind::Binary,
                ReceiveTime::from_unix_nanos(0),
                MonotonicElapsed::from_nanos(0),
                payload,
                1 << 20,
            )
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn a_small_slice_is_charged_for_its_whole_allocation() {
        let big = crate::kite::envelope::Payload::new(vec![7; 1 << 20]);
        let slice = big.slice(0..8).unwrap();
        drop(big);
        let event = raw(slice);
        assert_eq!(
            charge(&event),
            (1 << 20) + std::mem::size_of::<TickerEvent>()
        );
        // 1.5 MiB: room for one such event, not two.
        let (tx, mut rx) = queue(16, 3 << 19, None);
        tx.reserve(charge(&event)).await.unwrap().send(event);
        assert!(tx.retained() >= 1 << 20, "{}", tx.retained());
        assert_eq!(tx.len(), 1);
        assert!(tx.oldest().is_some());
        // A second one waits until the first is taken.
        let second = raw(crate::kite::envelope::Payload::new(vec![7; (1 << 20) - 64])
            .slice(0..8)
            .unwrap());
        let pending = tx.reserve(charge(&second));
        tokio::pin!(pending);
        assert!(futures_util::poll!(&mut pending).is_pending());
        let taken = std::future::poll_fn(|cx| rx.poll_recv(cx)).await.unwrap();
        assert!(matches!(taken, TickerEvent::Raw(_)));
        drop(taken);
        assert_eq!(tx.len(), 0);
        pending.await.unwrap().send(second);
        assert_eq!(tx.len(), 1);
    }

    #[tokio::test]
    async fn a_dropped_reservation_releases_its_room() {
        let (tx, _rx) = queue(16, 1 << 20, None);
        let full = tx.reserve(1 << 20).await.unwrap();
        let waiting = tx.reserve(1);
        drop(waiting);
        drop(full);
        assert_eq!(tx.retained(), 0);
        assert!(tx.reserve(1 << 20).await.is_ok());
    }
}
