//! Provenance adaptation: raw observations to owned decoded events.
//!
//! [`Adapter::decode`] turns one [`RawObservation`] into a [`Decoded`] batch.
//! Every event and diagnostic carries the observation's complete
//! [`SourceKey`], the packet index where there is one, and the decoder and
//! conversion versions ([`Versions`]). The observation is only borrowed:
//! decoding never consumes or alters the raw evidence, so it stays available
//! after any failure.
//!
//! Outputs depend only on the observation's payload, kind and source key
//! under pinned versions. A live observation and a captured copy of it
//! (for example one serialized and read back) therefore decode to equal
//! events; the [`SourceMode`] label is the only difference, and it reaches
//! metrics only. Bare payloads decode without any source identity through
//! [`crate::kite::decoder::framing`], [`crate::kite::decoder::packets`] and
//! [`crate::kite::decoder::text`].
//!
//! # Results
//!
//! | Outcome | `result` label |
//! |---|---|
//! | every packet, the heartbeat or the text message decoded | `ok` |
//! | the batch framed but some packets did not decode | `partial` |
//! | nothing decoded: framing, packet or text failure | `error` |
//! | a text message of a type this build does not know | `unknown_format` |
//!
//! A structurally invalid batch yields no packet at all. Packets that frame
//! but fail field decoding are reported one diagnostic each, beside the
//! packets that decoded.
//!
//! # Instrumentation
//!
//! The adapter wraps the pure parser. It counts exactly one
//! `manja_decode_batches_total` per observation, error and unknown outcomes
//! included, and times it in `manja_decode_duration_seconds` when a
//! recorder is attached; the parser itself is never instrumented. The
//! `manja.decode.batch` span is off unless [`Adapter::with_spans`] enables
//! it, and then at TRACE level. Source identifiers appear only as span
//! fields, never as labels. Semantic output needs no clock, network,
//! credential, file or runtime.
//!
use crate::kite::decoder::framing::{self, DecodeDiagnosticKind, FramingLimits, Message};
use crate::kite::decoder::packets::{self, Packet, PacketErrorKind};
use crate::kite::decoder::text::{self, TextEvent, TextLimits};
use crate::kite::envelope::{EnvelopeError, PayloadKind, RawObservation, SourceKey};
use crate::kite::obs::diagnostics::{BoundedText, DEFAULT_TEXT_BYTES};
use crate::kite::obs::handle::{Labels, Observability};
use crate::kite::obs::schema::{DecodeResult, PayloadKindLabel, SourceMode};
use crate::kite::protocol::CONVERSION_POLICY_VERSION;

/// Version of this decoder's output semantics.
pub const DECODER_VERSION: u32 = 1;

/// The versions an output was produced under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Versions {
    /// [`DECODER_VERSION`].
    pub decoder: u32,
    /// The price-conversion policy version.
    pub conversion: u32,
}

/// The versions of this build.
pub const VERSIONS: Versions = Versions {
    decoder: DECODER_VERSION,
    conversion: CONVERSION_POLICY_VERSION,
};

/// One decoded, owned event.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DecodedEvent {
    /// The one-byte heartbeat.
    Heartbeat {
        /// Source of the observation.
        source: SourceKey,
    },
    /// One packet of a binary message.
    Packet {
        /// Source of the observation.
        source: SourceKey,
        /// Position within the message.
        packet_index: u16,
        /// The decoded packet.
        packet: Packet,
    },
    /// A text message.
    Text {
        /// Source of the observation.
        source: SourceKey,
        /// The parsed message.
        event: TextEvent,
    },
}

/// A decoding problem, attached to its source.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeDiagnostic {
    /// Source of the observation.
    pub source: SourceKey,
    /// Versions in effect.
    pub versions: Versions,
    /// The packet concerned, if any.
    pub packet_index: Option<u16>,
    /// What went wrong.
    pub kind: DecodeDiagnosticKind,
    /// Bounded detail (`B-DIAG-02`).
    pub detail: BoundedText,
}

/// The outcome of decoding one observation.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Decoded {
    /// Versions in effect.
    pub versions: Versions,
    /// Decoded events, in payload order.
    pub events: Vec<DecodedEvent>,
    /// Problems, in payload order.
    pub diagnostics: Vec<DecodeDiagnostic>,
    /// The normalized result.
    pub result: DecodeResult,
}

/// An observation this build cannot decode at all.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdapterError {
    /// Its envelope version is not supported.
    UnsupportedEnvelope(EnvelopeError),
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedEnvelope(e) => write!(f, "cannot decode: {e}"),
        }
    }
}

impl std::error::Error for AdapterError {}

/// Decodes observations, with optional instrumentation.
#[derive(Clone, Debug)]
pub struct Adapter {
    framing: FramingLimits,
    text: TextLimits,
    mode: SourceMode,
    obs: Observability,
    spans: bool,
}

impl Adapter {
    /// An adapter for observations from `mode`, recording in `obs`.
    pub fn new(mode: SourceMode, obs: &Observability) -> Self {
        Self {
            framing: FramingLimits::default(),
            text: TextLimits::default(),
            mode,
            obs: obs.clone(),
            spans: false,
        }
    }

    /// Use these framing and text bounds.
    pub fn with_limits(mut self, framing: FramingLimits, text: TextLimits) -> Self {
        self.framing = framing;
        self.text = text;
        self
    }

    /// Emit a TRACE `manja.decode.batch` span per observation.
    pub fn with_spans(mut self, enabled: bool) -> Self {
        self.spans = enabled;
        self
    }

    /// Decode one observation. It is borrowed and never altered.
    pub fn decode(&self, observation: &RawObservation) -> Result<Decoded, AdapterError> {
        observation
            .version()
            .check_supported()
            .map_err(AdapterError::UnsupportedEnvelope)?;
        let started = self.obs.is_recording().then(std::time::Instant::now);
        let decoded = decode_observation(observation, self.framing, self.text);
        let kind = match observation.kind() {
            PayloadKind::Binary => PayloadKindLabel::Binary,
            PayloadKind::Text => PayloadKindLabel::Text,
        };
        self.obs
            .counter(Labels::decode_batch(self.mode, kind, decoded.result), 1);
        if let Some(started) = started {
            self.obs.histogram(
                Labels::decode_duration(self.mode, kind),
                started.elapsed().as_secs_f64(),
            );
        }
        if self.spans {
            tracing::trace_span!(
                "manja.decode.batch",
                source_key = %observation.source(),
                decoder_version = DECODER_VERSION,
                payload_kind = kind.as_str(),
                packet_count = decoded.events.len(),
                result = decoded.result.as_str(),
            )
            .in_scope(|| {});
        }
        Ok(decoded)
    }
}

// The pure part: no metrics, spans or clock.
fn decode_observation(o: &RawObservation, framing: FramingLimits, text: TextLimits) -> Decoded {
    let source = o.source();
    let mut events = Vec::new();
    let mut diagnostics = Vec::new();
    let diagnostic = |packet_index, kind, detail: &str| DecodeDiagnostic {
        source: source.clone(),
        versions: VERSIONS,
        packet_index,
        kind,
        detail: BoundedText::sanitize(detail, DEFAULT_TEXT_BYTES),
    };
    let result = match o.kind() {
        PayloadKind::Binary => match framing::frame(o.payload().as_bytes(), framing) {
            Ok(Message::Heartbeat) => {
                events.push(DecodedEvent::Heartbeat {
                    source: source.clone(),
                });
                DecodeResult::Ok
            }
            Ok(Message::Packets(frames)) => {
                for f in frames.iter() {
                    match packets::decode(&f) {
                        Ok(packet) => events.push(DecodedEvent::Packet {
                            source: source.clone(),
                            packet_index: f.index(),
                            packet,
                        }),
                        Err(e) => {
                            let kind = match e.kind {
                                PacketErrorKind::UnknownLength(_) => {
                                    DecodeDiagnosticKind::UnknownLength
                                }
                                _ => DecodeDiagnosticKind::InvalidField,
                            };
                            diagnostics.push(diagnostic(
                                Some(e.packet_index),
                                kind,
                                &e.to_string(),
                            ));
                        }
                    }
                }
                match (events.is_empty(), diagnostics.is_empty()) {
                    (_, true) => DecodeResult::Ok,
                    (false, false) => DecodeResult::Partial,
                    (true, false) => DecodeResult::Error,
                }
            }
            Err(e) => {
                diagnostics.push(diagnostic(e.packet_index(), e.kind(), &e.to_string()));
                DecodeResult::Error
            }
        },
        PayloadKind::Text => match o.text().map(|t| text::parse(t, text)) {
            Some(Ok(event)) => {
                let unknown = matches!(event, TextEvent::Unknown { .. });
                events.push(DecodedEvent::Text {
                    source: source.clone(),
                    event,
                });
                if unknown {
                    diagnostics.push(diagnostic(
                        None,
                        DecodeDiagnosticKind::UnknownTextType,
                        "a text message of a type this build does not know",
                    ));
                    DecodeResult::UnknownFormat
                } else {
                    DecodeResult::Ok
                }
            }
            Some(Err(e)) => {
                diagnostics.push(diagnostic(None, e.kind(), e.detail().as_str()));
                DecodeResult::Error
            }
            None => {
                diagnostics.push(diagnostic(
                    None,
                    DecodeDiagnosticKind::InvalidText,
                    "the text payload is not UTF-8",
                ));
                DecodeResult::Error
            }
        },
    };
    Decoded {
        versions: VERSIONS,
        events,
        diagnostics,
        result,
    }
}
