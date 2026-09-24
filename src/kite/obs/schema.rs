//! Frozen observability schema, version [`OBS_SCHEMA_VERSION`].
//!
//! Instrument names, types, units and exact label sets; span names and
//! fields; closed label-value domains; and the shared histogram bucket
//! profile. These are versioned compatibility surfaces: an addition, such as
//! a new value in a label domain, is a documented minor change, while a
//! rename, unit change or label-set change needs a schema version bump. The
//! domain enums are `#[non_exhaustive]` so that adding a value breaks no
//! downstream code. `tests/obs_schema/` snapshots
//! [`catalogue_text`].
//!
//! Every label value comes from a closed enum below, so a metric series can
//! never carry an account or API key, token, order or intent ID, epoch,
//! sequence, URL, path or broker string. Unknown inputs collapse to the
//! domain's `unknown` or error value.
//!
use std::fmt::Write as _;

/// Observability schema version.
pub const OBS_SCHEMA_VERSION: u32 = 1;

/// Shared histogram bucket upper bounds, in seconds.
pub const HISTOGRAM_BUCKETS_SECONDS: [f64; 15] = [
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];

/// Maximum number of labels any instrument carries.
pub const MAX_LABELS: usize = 4;

/// Fixed length bound of every label value, in bytes (`B-DIAG-04`).
pub const MAX_LABEL_VALUE_BYTES: usize = 64;

macro_rules! domain {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$doc])*
        ///
        /// A later release may add values, so a `match` outside this crate
        /// needs a wildcard arm; [`Self::ALL`] lists every value of this build.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum $name {
            $($(#[$vdoc])* $variant,)+
        }

        impl $name {
            /// Every value of the domain, in catalogue order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The label value.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $s,)+
                }
            }
        }
    };
}

domain!(
    /// Label key.
    LabelKey {
        /// HTTP method.
        Method => "method",
        /// Endpoint template.
        Endpoint => "endpoint",
        /// Quota (retry) class.
        QuotaClass => "quota_class",
        /// Normalized result.
        Result => "result",
        /// Retry cause.
        ErrorClass => "error_class",
        /// Admission decision.
        AdmissionResult => "admission_result",
        /// Transport that observed a rejection.
        Transport => "transport",
        /// Reconnect reason.
        Reason => "reason",
        /// Ticker command kind.
        Command => "command",
        /// Command decision.
        Decision => "decision",
        /// Payload kind.
        PayloadKind => "payload_kind",
        /// SDK queue role.
        QueueRole => "queue_role",
        /// Decoder source mode.
        SourceMode => "source_mode",
    }
);

domain!(
    /// HTTP method.
    Method {
        /// GET.
        Get => "GET",
        /// POST.
        Post => "POST",
        /// PUT.
        Put => "PUT",
        /// DELETE.
        Delete => "DELETE",
    }
);

domain!(
    /// Endpoint template. Dynamic path segments never appear.
    Endpoint {
        /// `/user/profile`
        UserProfile => "/user/profile",
        /// `/user/margins`
        UserMargins => "/user/margins",
        /// `/user/margins/{segment}`
        UserMarginsSegment => "/user/margins/{segment}",
        /// `/orders`
        Orders => "/orders",
        /// `/orders/{variety}`
        OrdersVariety => "/orders/{variety}",
        /// `/orders/{variety}/{order_id}`
        OrdersVarietyId => "/orders/{variety}/{order_id}",
        /// `/orders/{order_id}`
        OrdersId => "/orders/{order_id}",
        /// `/orders/{order_id}/trades`
        OrdersIdTrades => "/orders/{order_id}/trades",
        /// `/trades`
        Trades => "/trades",
        /// `/portfolio/holdings`
        Holdings => "/portfolio/holdings",
        /// `/portfolio/holdings/auctions`
        HoldingsAuctions => "/portfolio/holdings/auctions",
        /// `/portfolio/holdings/authorise`
        HoldingsAuthorise => "/portfolio/holdings/authorise",
        /// `/portfolio/positions`
        Positions => "/portfolio/positions",
        /// `/instruments`
        Instruments => "/instruments",
        /// `/instruments/{exchange}`
        InstrumentsExchange => "/instruments/{exchange}",
        /// `/instruments/historical/{instrument_token}/{interval}`
        InstrumentsHistorical => "/instruments/historical/{instrument_token}/{interval}",
        /// `/quote`
        Quote => "/quote",
        /// `/quote/ohlc`
        QuoteOhlc => "/quote/ohlc",
        /// `/quote/ltp`
        QuoteLtp => "/quote/ltp",
        /// `/margins/orders`
        MarginsOrders => "/margins/orders",
        /// `/margins/basket`
        MarginsBasket => "/margins/basket",
        /// `/charges/orders`
        ChargesOrders => "/charges/orders",
        /// `/session/token`
        SessionToken => "/session/token",
        /// `/mf/orders`
        MfOrders => "/mf/orders",
        /// `/mf/orders/{order_id}`
        MfOrdersId => "/mf/orders/{order_id}",
        /// `/mf/sips`
        MfSips => "/mf/sips",
        /// `/mf/holdings`
        MfHoldings => "/mf/holdings",
        /// `/mf/instruments`
        MfInstruments => "/mf/instruments",
        /// `/gtt/triggers`
        GttTriggers => "/gtt/triggers",
        /// `/gtt/triggers/{id}`
        GttTriggersId => "/gtt/triggers/{id}",
        /// Any other endpoint.
        Unknown => "unknown",
    }
);

domain!(
    /// Quota (retry-policy) class of an operation.
    QuotaClass {
        /// Idempotent read.
        Read => "read",
        /// Read-like POST calculation.
        Calc => "calc",
        /// Trading mutation.
        Mut => "mut",
        /// Session operation.
        Sess => "sess",
    }
);

domain!(
    /// Normalized result of a logical HTTP operation.
    HttpOperationResult {
        /// Success.
        Ok => "ok",
        /// Non-success HTTP status with no broker classification.
        HttpStatus => "http_status",
        /// Broker error envelope.
        BrokerError => "broker_error",
        /// Credentials rejected.
        AuthRejected => "auth_rejected",
        /// Transport failure.
        TransportError => "transport_error",
        /// Attempt timeout.
        Timeout => "timeout",
        /// Operation deadline expired.
        Deadline => "deadline",
        /// Admission rejected before transport.
        AdmissionRejected => "admission_rejected",
        /// Cancelled.
        Cancelled => "cancelled",
        /// Response could not be decoded.
        DecodeError => "decode_error",
        /// Local validation failed before transport.
        Validation => "validation",
    }
);

domain!(
    /// Normalized result of one actual HTTP attempt.
    HttpAttemptResult {
        /// Success.
        Ok => "ok",
        /// Non-success HTTP status.
        HttpStatus => "http_status",
        /// Broker error envelope.
        BrokerError => "broker_error",
        /// Credentials rejected.
        AuthRejected => "auth_rejected",
        /// Transport failure.
        TransportError => "transport_error",
        /// Attempt timeout.
        Timeout => "timeout",
        /// Cancelled during the attempt.
        Cancelled => "cancelled",
        /// Response could not be decoded.
        DecodeError => "decode_error",
    }
);

domain!(
    /// Why a retry attempt started.
    RetryCause {
        /// HTTP 429.
        Http429 => "http_429",
        /// HTTP 5xx.
        Http5xx => "http_5xx",
        /// Transport failure.
        TransportError => "transport_error",
        /// Attempt timeout.
        Timeout => "timeout",
    }
);

domain!(
    /// Outcome of one admission wait.
    AdmissionResult {
        /// Capacity granted.
        Granted => "granted",
        /// Rejected: waiter bound or wait bound reached.
        Rejected => "rejected",
        /// The waiting operation was cancelled.
        Cancelled => "cancelled",
        /// The operation deadline expired while waiting.
        Deadline => "deadline",
    }
);

domain!(
    /// Transport that observed a credential rejection.
    TransportLabel {
        /// HTTP.
        Http => "http",
        /// Ticker WebSocket.
        Ticker => "ticker",
    }
);

domain!(
    /// Result of one ticker connection attempt.
    ConnectionResult {
        /// Handshake succeeded.
        Ok => "ok",
        /// Credentials rejected.
        AuthRejected => "auth_rejected",
        /// Other non-success handshake status.
        HttpStatus => "http_status",
        /// Transport failure.
        TransportError => "transport_error",
        /// Handshake timeout.
        Timeout => "timeout",
        /// Cancelled by shutdown.
        Cancelled => "cancelled",
    }
);

domain!(
    /// Why a reconnect attempt began.
    ReconnectReason {
        /// Stream ended without a close frame.
        Eof => "eof",
        /// Peer sent a close frame.
        RemoteClose => "remote_close",
        /// Liveness timeout.
        LivenessTimeout => "liveness_timeout",
        /// Transport failure.
        TransportError => "transport_error",
        /// Protocol violation.
        ProtocolError => "protocol_error",
    }
);

domain!(
    /// Ticker command kind.
    CommandKind {
        /// Subscribe with a mode.
        Subscribe => "subscribe",
        /// Unsubscribe.
        Unsubscribe => "unsubscribe",
        /// Change mode.
        SetMode => "set_mode",
        /// Replace the whole desired map.
        Replace => "replace",
    }
);

domain!(
    /// Command decision by the source owner.
    Decision {
        /// Accepted and assigned a revision.
        Accepted => "accepted",
        /// Rejected by validation.
        Rejected => "rejected",
    }
);

domain!(
    /// Result of one subscription restoration.
    RestoreResult {
        /// Commands written.
        Sent => "sent",
        /// Writing failed.
        SendFailed => "send_failed",
        /// Replaced by a later revision.
        Superseded => "superseded",
        /// Cancelled by shutdown or disconnect.
        Cancelled => "cancelled",
    }
);

domain!(
    /// WebSocket payload kind.
    PayloadKindLabel {
        /// Binary message.
        Binary => "binary",
        /// Text message.
        Text => "text",
    }
);

domain!(
    /// SDK-managed queue role.
    QueueRole {
        /// Ticker command mailbox.
        Command => "command",
        /// Primary raw-delivery queue.
        RawPrimary => "raw_primary",
        /// Best-effort notification stream.
        Notification => "notification",
        /// In-progress socket send.
        PendingSend => "pending_send",
    }
);

domain!(
    /// Decoder invocation mode.
    SourceMode {
        /// Live ticker observations.
        Live => "live",
        /// Previously captured observations.
        Replay => "replay",
        /// Bare payloads without a source.
        Standalone => "standalone",
    }
);

domain!(
    /// Result of one decoder adapter invocation.
    DecodeResult {
        /// Every packet decoded.
        Ok => "ok",
        /// Some packets decoded, some produced diagnostics.
        Partial => "partial",
        /// Structurally invalid input.
        Error => "error",
        /// Unknown message format.
        UnknownFormat => "unknown_format",
    }
);

domain!(
    /// Result of a ticker shutdown.
    ShutdownResult {
        /// Clean shutdown within the deadline.
        Clean => "clean",
        /// The shutdown deadline expired.
        DeadlineExpired => "deadline_expired",
        /// Shutdown ended in a failure.
        Failed => "failed",
        /// The owner task panicked.
        Panicked => "panicked",
    }
);

/// Metric instrument type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InstrumentKind {
    /// Monotonic counter.
    Counter,
    /// Gauge.
    Gauge,
    /// Histogram over [`HISTOGRAM_BUCKETS_SECONDS`].
    Histogram,
}

impl InstrumentKind {
    /// Catalogue name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

macro_rules! instruments {
    ($($variant:ident => $name:literal, $kind:ident, $unit:literal, [$($key:ident),*];)+) => {
        /// A metric instrument of the frozen catalogue.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Instrument {
            $(
                #[doc = concat!("`", $name, "`")]
                $variant,
            )+
        }

        impl Instrument {
            /// Every instrument, in catalogue order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Stable instrument name.
            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name,)+ }
            }

            /// Instrument type.
            pub const fn kind(self) -> InstrumentKind {
                match self { $(Self::$variant => InstrumentKind::$kind,)+ }
            }

            /// Unit.
            pub const fn unit(self) -> &'static str {
                match self { $(Self::$variant => $unit,)+ }
            }

            /// Exact label keys, in order.
            pub const fn label_keys(self) -> &'static [LabelKey] {
                match self { $(Self::$variant => &[$(LabelKey::$key),*],)+ }
            }
        }
    };
}

instruments! {
    HttpOperationsTotal => "manja_http_operations_total", Counter, "operations", [Method, Endpoint, QuotaClass, Result];
    HttpOperationDuration => "manja_http_operation_duration_seconds", Histogram, "seconds", [Method, Endpoint, QuotaClass, Result];
    HttpAttemptsTotal => "manja_http_attempts_total", Counter, "attempts", [Method, Endpoint, Result];
    HttpAttemptDuration => "manja_http_attempt_duration_seconds", Histogram, "seconds", [Method, Endpoint, Result];
    HttpInFlight => "manja_http_in_flight", Gauge, "attempts", [QuotaClass];
    HttpAdmissionWaiters => "manja_http_admission_waiters", Gauge, "waiters", [QuotaClass];
    HttpAdmissionWait => "manja_http_admission_wait_seconds", Histogram, "seconds", [QuotaClass, AdmissionResult];
    HttpRetriesTotal => "manja_http_retries_total", Counter, "retries", [Method, Endpoint, ErrorClass];
    HttpRejectedRowsTotal => "manja_http_rejected_rows_total", Counter, "rows", [Endpoint];
    AuthRejectionsTotal => "manja_auth_rejections_total", Counter, "rejections", [Transport];
    TickerConnectionAttemptsTotal => "manja_ticker_connection_attempts_total", Counter, "attempts", [Result];
    TickerConnectDuration => "manja_ticker_connect_duration_seconds", Histogram, "seconds", [Result];
    TickerConnectionsActive => "manja_ticker_connections_active", Gauge, "connections", [];
    TickerReconnectsTotal => "manja_ticker_reconnects_total", Counter, "reconnect attempts", [Reason];
    TickerCommandsTotal => "manja_ticker_commands_total", Counter, "decisions", [Command, Decision];
    TickerRestoreDuration => "manja_ticker_restore_duration_seconds", Histogram, "seconds", [Result];
    TickerReceivedMessagesTotal => "manja_ticker_received_messages_total", Counter, "messages", [PayloadKind];
    TickerReceivedBytesTotal => "manja_ticker_received_bytes_total", Counter, "bytes", [PayloadKind];
    SdkQueueMessages => "manja_sdk_queue_messages", Gauge, "messages", [QueueRole];
    SdkQueueRetainedBytes => "manja_sdk_queue_retained_bytes", Gauge, "bytes", [QueueRole];
    SdkQueueOldestAge => "manja_sdk_queue_oldest_age_seconds", Gauge, "seconds", [QueueRole];
    DecodeBatchesTotal => "manja_decode_batches_total", Counter, "batches", [SourceMode, PayloadKind, Result];
    DecodeDuration => "manja_decode_duration_seconds", Histogram, "seconds", [SourceMode, PayloadKind];
    TickerShutdownDuration => "manja_ticker_shutdown_duration_seconds", Histogram, "seconds", [Result];
    TelemetryDroppedRecordsTotal => "manja_telemetry_dropped_records_total", Counter, "records", [];
}

/// Size of the closed value domain of `key` for `instrument`. `result` has a
/// different domain per instrument.
pub fn domain_size(instrument: Instrument, key: LabelKey) -> usize {
    use Instrument as I;
    match key {
        LabelKey::Method => Method::ALL.len(),
        LabelKey::Endpoint => Endpoint::ALL.len(),
        LabelKey::QuotaClass => QuotaClass::ALL.len(),
        LabelKey::ErrorClass => RetryCause::ALL.len(),
        LabelKey::AdmissionResult => AdmissionResult::ALL.len(),
        LabelKey::Transport => TransportLabel::ALL.len(),
        LabelKey::Reason => ReconnectReason::ALL.len(),
        LabelKey::Command => CommandKind::ALL.len(),
        LabelKey::Decision => Decision::ALL.len(),
        LabelKey::PayloadKind => PayloadKindLabel::ALL.len(),
        LabelKey::QueueRole => QueueRole::ALL.len(),
        LabelKey::SourceMode => SourceMode::ALL.len(),
        LabelKey::Result => match instrument {
            I::HttpOperationsTotal | I::HttpOperationDuration => HttpOperationResult::ALL.len(),
            I::HttpAttemptsTotal | I::HttpAttemptDuration => HttpAttemptResult::ALL.len(),
            I::TickerConnectionAttemptsTotal | I::TickerConnectDuration => {
                ConnectionResult::ALL.len()
            }
            I::TickerRestoreDuration => RestoreResult::ALL.len(),
            I::DecodeBatchesTotal => DecodeResult::ALL.len(),
            I::TickerShutdownDuration => ShutdownResult::ALL.len(),
            _ => 0,
        },
    }
}

/// Upper bound on the number of series `instrument` can produce within one
/// recording scope: the product of its label-domain sizes.
pub fn series_bound(instrument: Instrument) -> usize {
    instrument
        .label_keys()
        .iter()
        .map(|k| domain_size(instrument, *k))
        .product()
}

/// A span of the frozen catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpanSpec {
    /// Span name.
    pub name: &'static str,
    /// `tracing` level.
    pub level: &'static str,
    /// Required fields.
    pub fields: &'static [&'static str],
}

/// The frozen span catalogue.
pub const SPANS: &[SpanSpec] = &[
    SpanSpec {
        name: "manja.http.operation",
        level: "DEBUG",
        fields: &[
            "operation_id",
            "method",
            "endpoint",
            "quota_class",
            "deadline_ms",
            "result",
            "stage",
        ],
    },
    SpanSpec {
        name: "manja.http.admission",
        level: "DEBUG",
        fields: &["operation_id", "quota_class", "wait_ms", "admission_result"],
    },
    SpanSpec {
        name: "manja.http.attempt",
        level: "DEBUG",
        fields: &[
            "operation_id",
            "attempt",
            "method",
            "endpoint",
            "http_status",
            "error_class",
            "stage",
        ],
    },
    SpanSpec {
        name: "manja.ticker.connection",
        level: "DEBUG",
        fields: &["feed_id", "connection_epoch", "reason", "result"],
    },
    SpanSpec {
        name: "manja.ticker.restore",
        level: "DEBUG",
        fields: &[
            "connection_epoch",
            "desired_revision",
            "instrument_count",
            "sent_count",
            "result",
        ],
    },
    SpanSpec {
        name: "manja.ticker.command",
        level: "DEBUG",
        fields: &["command", "revision", "decision", "rejection"],
    },
    SpanSpec {
        name: "manja.decode.batch",
        level: "TRACE",
        fields: &[
            "source_key",
            "decoder_version",
            "payload_kind",
            "packet_count",
            "result",
        ],
    },
    SpanSpec {
        name: "manja.ticker.shutdown",
        level: "DEBUG",
        fields: &["reason", "elapsed_ms", "pending_deliveries", "result"],
    },
];

/// Render the whole catalogue as stable text, for schema snapshots.
pub fn catalogue_text() -> String {
    let mut out = String::new();
    let _ = writeln!(out, "obs_schema_version {OBS_SCHEMA_VERSION}");
    let _ = writeln!(
        out,
        "histogram_buckets_seconds {HISTOGRAM_BUCKETS_SECONDS:?}"
    );
    let _ = writeln!(out, "max_label_value_bytes {MAX_LABEL_VALUE_BYTES}");
    let _ = writeln!(out);
    for i in Instrument::ALL {
        let keys: Vec<_> = i.label_keys().iter().map(|k| k.as_str()).collect();
        let _ = writeln!(
            out,
            "metric {} {} unit={} labels=[{}] series_bound={}",
            i.name(),
            i.kind().as_str(),
            i.unit(),
            keys.join(","),
            series_bound(*i)
        );
    }
    let _ = writeln!(out);
    for s in SPANS {
        let _ = writeln!(
            out,
            "span {} level={} fields=[{}]",
            s.name,
            s.level,
            s.fields.join(",")
        );
    }
    let _ = writeln!(out);
    macro_rules! dump {
        ($($label:literal => $ty:ident),+) => {$(
            let values: Vec<_> = $ty::ALL.iter().map(|v| v.as_str()).collect();
            let _ = writeln!(out, "domain {} [{}]", $label, values.join(","));
        )+};
    }
    dump!(
        "method" => Method,
        "endpoint" => Endpoint,
        "quota_class" => QuotaClass,
        "result.http_operation" => HttpOperationResult,
        "result.http_attempt" => HttpAttemptResult,
        "error_class" => RetryCause,
        "admission_result" => AdmissionResult,
        "transport" => TransportLabel,
        "result.ticker_connection" => ConnectionResult,
        "reason" => ReconnectReason,
        "command" => CommandKind,
        "decision" => Decision,
        "result.restore" => RestoreResult,
        "payload_kind" => PayloadKindLabel,
        "queue_role" => QueueRole,
        "source_mode" => SourceMode,
        "result.decode" => DecodeResult,
        "result.shutdown" => ShutdownResult
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_bounds_match_the_contract_table() {
        use Instrument as I;
        let expected = [
            (I::HttpOperationsTotal, 5456),
            (I::HttpOperationDuration, 5456),
            (I::HttpAttemptsTotal, 992),
            (I::HttpAttemptDuration, 992),
            (I::HttpInFlight, 4),
            (I::HttpAdmissionWaiters, 4),
            (I::HttpAdmissionWait, 16),
            (I::HttpRetriesTotal, 496),
            (I::HttpRejectedRowsTotal, 31),
            (I::AuthRejectionsTotal, 2),
            (I::TickerConnectionAttemptsTotal, 6),
            (I::TickerConnectDuration, 6),
            (I::TickerConnectionsActive, 1),
            (I::TickerReconnectsTotal, 5),
            (I::TickerCommandsTotal, 8),
            (I::TickerRestoreDuration, 4),
            (I::TickerReceivedMessagesTotal, 2),
            (I::TickerReceivedBytesTotal, 2),
            (I::SdkQueueMessages, 4),
            (I::SdkQueueRetainedBytes, 4),
            (I::SdkQueueOldestAge, 4),
            (I::DecodeBatchesTotal, 24),
            (I::DecodeDuration, 6),
            (I::TickerShutdownDuration, 4),
            (I::TelemetryDroppedRecordsTotal, 1),
        ];
        assert_eq!(expected.len(), Instrument::ALL.len());
        for (i, bound) in expected {
            assert_eq!(series_bound(i), bound, "{}", i.name());
        }
    }

    #[test]
    fn label_values_are_short_and_closed() {
        assert!(Endpoint::ALL
            .iter()
            .all(|e| e.as_str().len() <= MAX_LABEL_VALUE_BYTES));
        assert_eq!(Endpoint::ALL.len(), 31);
        assert!(Instrument::ALL
            .iter()
            .all(|i| i.label_keys().len() <= MAX_LABELS));
    }
}
