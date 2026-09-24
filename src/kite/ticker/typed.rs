//! Typed ticker events: the actor ticker composed with the decoder.
//!
//! Compiled with both `ticker` and `decoder`. [`TypedEvents`] wraps the one
//! primary receiver of a ticker and decodes each observation with an
//! [`Adapter`] as it passes. The raw observation is always yielded beside
//! its decoded form, never replaced by it, and no second socket or task is
//! involved: dropping `TypedEvents` drops the primary receiver, with the
//! same consequences. End of stream, the terminal error and cancellation
//! behave exactly as on [`TickerEvents`].
//!
//! ```no_run
//! use futures_util::StreamExt;
//! use manja::kite::connect::credentials::Credentials;
//! use manja::kite::decoder::adapter::Adapter;
//! use manja::kite::obs::{schema::SourceMode, Observability};
//! use manja::kite::ticker::actor::owner::TickerBuilder;
//! use manja::kite::ticker::typed::{TypedEvent, TypedEvents};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let obs = Observability::disabled();
//! let credentials = Credentials::new("api_key", "access_token")?;
//! let (handle, events, _guard) = TickerBuilder::new(credentials)
//!     .observability(obs.clone())
//!     .spawn()?;
//! let mut typed = TypedEvents::new(events, Adapter::new(SourceMode::Live, &obs));
//! while let Some(item) = typed.next().await {
//!     match item? {
//!         TypedEvent::Observation { raw, decoded } => {
//!             let _ = (raw.source(), decoded);
//!         }
//!         TypedEvent::Lifecycle(event) => {
//!             let _ = event.kind();
//!         }
//!         _ => {}
//!     }
//! }
//! # let _ = handle;
//! # Ok(()) }
//! ```
//!
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;

use crate::kite::decoder::adapter::{Adapter, AdapterError, Decoded};
use crate::kite::envelope::{LifecycleEvent, RawObservation};
use crate::kite::ticker::actor::owner::{TickerError, TickerEvent, TickerEvents};

/// One item of a typed ticker stream.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TypedEvent {
    /// A source-lifecycle event, as on the primary stream.
    Lifecycle(LifecycleEvent),
    /// An observation and its decoding.
    Observation {
        /// The observation exactly as received.
        raw: RawObservation,
        /// What the adapter made of it.
        decoded: Result<Decoded, AdapterError>,
    },
}

/// The primary ticker stream, decoded as it passes.
///
/// `Send + Unpin + 'static`, like [`TickerEvents`], and likewise not
/// `Clone`.
#[derive(Debug)]
pub struct TypedEvents {
    events: TickerEvents,
    adapter: Adapter,
}

impl TypedEvents {
    /// Decode `events` with `adapter`.
    pub fn new(events: TickerEvents, adapter: Adapter) -> Self {
        Self { events, adapter }
    }

    /// The undecoded primary stream back.
    pub fn into_raw(self) -> TickerEvents {
        self.events
    }
}

impl Stream for TypedEvents {
    type Item = Result<TypedEvent, TickerError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        Pin::new(&mut this.events).poll_next(cx).map(|item| {
            item.map(|item| {
                item.map(|event| match event {
                    TickerEvent::Raw(raw) => {
                        let decoded = this.adapter.decode(&raw);
                        TypedEvent::Observation { raw, decoded }
                    }
                    TickerEvent::Lifecycle(l) => TypedEvent::Lifecycle(l),
                })
            })
        })
    }
}
