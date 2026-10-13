//! Downstream-style reachability: every public signature's
//! request, response, error, command, status, guard and bound types are
//! named here through their documented public paths. It compiles against
//! the default features and touches no network.

// The imports are the check: each must resolve through its public path.
#![allow(dead_code, unused_imports)]

use manja::kite::connect::admission::{
    Admission, AdmissionError, AdmissionGrant, AdmissionLimits, Headroom, Priority, QuotaProfile,
    RateClass, Window,
};
use manja::kite::connect::api::{
    Charges, Gtt, Margins, Market, MutualFunds, Orders, Portfolio, Session, User,
};
use manja::kite::connect::client::{HTTPClient, HttpClientBuilder, HttpDiagnostics, HttpFailure};
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::{
    AccessToken, ApiKey, ApiSecret, CredentialError, Credentials, RequestToken,
};
use manja::kite::connect::dispatch::{
    CloseError, Dispatched, Outcome, OutcomeStream, OutcomeStreamError, ReceiptIds,
};
use manja::kite::connect::models::{
    Candle, CandleInterval, DividendType, Exchange, FullQuote, GttCondition, GttOrder,
    GttOrderOutcome, GttOrderRequest, GttOrderResult, GttReceipt, GttRequest, GttStatus,
    GttTrigger, GttType, HistoricalData, HistoricalRequest, Holding, HoldingsAuthorisation,
    HoldingsAuthorisationRequest, IngressStamp, Instrument, LTPQuote, MfHolding, MfInstrument,
    MfOrder, MfOrderStatus, MfOrderVariety, MfPlan, MfPurchaseType, MfSip, ModifyOrderRequest,
    OHLCQuote, Order, OrderReceipt, PlaceOrderRequest, Position, PositionConversionRequest,
    Positions, QuoteMode, Quotes, RequestError, Response, ResponseMeta, Row, RowError, Rows,
    SchemeType, SipFrequency, SipStatus, Trade, UserSession,
};
use manja::kite::connect::scheduler::SchedulerLimits;
use manja::kite::connect::wire::WireForm;
use manja::kite::decoder::adapter::{Adapter, AdapterError, Decoded, DecodedEvent, Versions};
use manja::kite::decoder::framing::{Frame, FramingError, FramingLimits, Message, PacketFamily};
use manja::kite::decoder::packets::{Depth, DepthEntry, Packet, PacketError};
use manja::kite::decoder::text::{TextError, TextEvent, TextLimits};
use manja::kite::envelope::{
    GapFacts, LifecycleEvent, LifecycleKind, Payload, RawObservation, SourceIdentity, SourceKey,
};
use manja::kite::error::{BrokerError, HttpError, HttpErrorKind, ManjaError, TransportStage};
use manja::kite::obs::schema::{Endpoint, HttpOperationResult, Method};
use manja::kite::obs::{
    BridgeRecorder, InMemoryRecorder, Instrument as MetricInstrument, MetricRecorder, Observability,
};
use manja::kite::protocol::{
    Inbound, InstrumentToken, MfOrderId, OrderId, OrderIdError, OrderUpdate, Quantity,
    SESSION_EXPIRY_IST, ScaledPrice, session_expires_at,
};
use manja::kite::ticker::actor::lifecycle::ReconnectLimits;
use manja::kite::ticker::actor::owner::{
    CommandError, TaskGuard, TaskOutcome, TerminalReason, TickerBuilder, TickerError, TickerEvent,
    TickerEvents, TickerHandle, TickerLimits, TickerSpawnError, TickerState,
};
use manja::kite::ticker::actor::status::{QueueStatus, TickerFailure, TickerStatus};
use manja::kite::ticker::actor::subscriptions::{
    DesiredSubscriptions, Revision, SubscriptionCommand, SubscriptionError,
};
use manja::kite::ticker::typed::{TypedEvent, TypedEvents};
use manja::kite::ticker::{Mode, TickerRequest};

// Every resource is reachable from a shared reference, portfolio included,
// and the old `&mut` call shapes still compile.
fn resources(c: &HTTPClient, key: ApiKey) {
    let _: User<'_> = c.user();
    let _: Orders<'_> = c.orders();
    let _: Portfolio<'_> = c.portfolio();
    let _: Market<'_> = c.market();
    let _: Margins<'_> = c.margins();
    let _: Charges<'_> = c.charges();
    let _: Gtt<'_> = c.gtt();
    let _: MutualFunds<'_> = c.mutual_funds();
    let _: Session<'_> = c.session(key);
}

fn old_call_shapes(mut c: HTTPClient) {
    let m = &mut c;
    let _ = m.orders();
    let _ = m.market();
}

// Signatures of the main async operations, by type.
async fn signatures(c: &HTTPClient, h: &TickerHandle) {
    let _: Result<Response<Vec<Order>>, ManjaError> = c.orders().list_orders().await;
    if let Ok(book) = c.orders().list_orders().await {
        let meta: ResponseMeta = book.meta.clone();
        let _: (Option<String>, u16, u64, u32) = (
            meta.message,
            meta.http_status,
            meta.operation_id,
            meta.attempt,
        );
        let _: Response<usize> = book.map(|orders| orders.len());
    }
    let id: OrderId = "151220000000000".parse().unwrap();
    let _: Result<Response<Vec<Order>>, ManjaError> = c.orders().get_order_history(&id).await;
    let _: Result<Response<Vec<Trade>>, ManjaError> = c.orders().get_order_trades(&id).await;
    let _: Dispatched<OrderReceipt> = c
        .orders()
        .cancel_order(manja::kite::connect::models::OrderVariety::Regular, &id);
    let _: Result<OrderId, OrderIdError> = OrderId::new("1");
    let tag: Result<manja::kite::connect::models::Tag, RequestError> =
        manja::kite::connect::models::Tag::new("rebalance42");
    if let Ok(tag) = tag {
        let _: (&str, String) = (tag.as_str(), tag.to_string());
        let _: Result<Response<Vec<Order>>, ManjaError> = c.orders().list_orders_tagged(&tag).await;
    }
    let _: Result<manja::kite::connect::models::Tag, RequestError> = "a".parse();
    let _: Result<Response<Rows<Order>>, ManjaError> =
        c.orders().list_orders_with_rejections().await;
    let _: Result<Response<Rows<Order>>, ManjaError> =
        c.orders().get_order_history_with_rejections(&id).await;
    let _: Result<Response<Rows<Trade>>, ManjaError> =
        c.orders().list_trades_with_rejections().await;
    let _: Result<Response<Rows<Trade>>, ManjaError> =
        c.orders().get_order_trades_with_rejections(&id).await;
    let _: fn(&RowError) -> usize = |e| e.index;
    let _: fn(Row<Trade>) -> Option<Trade> = |r| match r {
        Row::Decoded(t) => Some(t),
        _ => None,
    };
    let _: Result<Response<Vec<Holding>>, ManjaError> = c.portfolio().get_holdings().await;
    let _: Result<Response<Positions>, ManjaError> = c.portfolio().get_positions().await;
    let gtt: GttRequest = GttRequest::single(
        Exchange::NSE,
        "INFY",
        702.0,
        798.0,
        GttOrderRequest::limit(
            manja::kite::connect::models::TransactionType::BUY,
            Quantity::new(1).unwrap(),
            manja::kite::connect::models::ProductType::CashAndCarry,
            702.5,
        ),
    );
    let _: Result<(), RequestError> = gtt.validate();
    let _: fn(&GttTrigger) -> Result<GttRequest, RequestError> = GttRequest::from_trigger;
    let _: Dispatched<GttReceipt> = c.gtt().place_trigger(&gtt);
    let _: Dispatched<GttReceipt> = c.gtt().modify_trigger(1, &gtt);
    let _: Dispatched<GttReceipt> = c.gtt().delete_trigger(1);
    let _: Result<Response<Vec<GttTrigger>>, ManjaError> = c.gtt().list_triggers().await;
    let _: Result<Response<GttTrigger>, ManjaError> = c.gtt().get_trigger(1).await;
    let _: Result<Response<Quotes<LTPQuote>>, ManjaError> =
        c.market().get_quotes::<LTPQuote>(&["NSE:INFY"]).await;
    let _: Result<Vec<Instrument>, ManjaError> = c.market().get_instruments_all().await;
    let at = chrono::DateTime::parse_from_rfc3339("2017-12-15T09:15:00+05:30").unwrap();
    let history = HistoricalRequest::new(InstrumentToken::new(5633), CandleInterval::Day, at, at);
    let _: Result<Response<HistoricalData>, ManjaError> = c.market().get_historical(&history).await;
    let _: fn(&HistoricalData) -> &Vec<Candle> = |d| &d.candles;
    let _: Dispatched<HoldingsAuthorisation> = c
        .portfolio()
        .authorise_holdings(&HoldingsAuthorisationRequest::all());
    let _: fn(&HoldingsAuthorisation, &ApiKey) -> String = HoldingsAuthorisation::portal_url;
    let _: fn(&HttpError) -> bool = HttpError::requires_holdings_authorisation;
    let mf = c.mutual_funds();
    let _: Result<Response<Vec<MfOrder>>, ManjaError> = mf.list_orders().await;
    let _: Result<Response<MfOrder>, ManjaError> =
        mf.get_order(&"1".parse::<MfOrderId>().unwrap()).await;
    let _: Result<Response<Vec<MfSip>>, ManjaError> = mf.list_sips().await;
    let _: Result<Response<Vec<MfHolding>>, ManjaError> = mf.list_holdings().await;
    let _: Result<Vec<MfInstrument>, ManjaError> = mf.get_instruments().await;
    let _: Result<String, ManjaError> = mf.get_instruments_csv().await;
    let _: HttpDiagnostics = c.diagnostics();
    // Wire forms of every mutation, with and without a request type.
    let place = PlaceOrderRequest::new(
        manja::kite::connect::models::OrderVariety::Regular,
        Exchange::NSE,
        "INFY",
        manja::kite::connect::models::TransactionType::BUY,
        manja::kite::connect::models::OrderType::Market,
        Quantity::new(1).unwrap(),
        manja::kite::connect::models::ProductType::CashAndCarry,
        manja::kite::connect::models::Tag::new("t1").unwrap(),
    );
    let wire: WireForm = place.wire().unwrap();
    let _: (
        manja::kite::obs::schema::Method,
        manja::kite::obs::schema::Endpoint,
        &str,
        Option<&str>,
        usize,
    ) = (
        wire.method(),
        wire.endpoint(),
        wire.path(),
        wire.content_type(),
        wire.body_len(),
    );
    let _: ([u8; 32], String, String) = (wire.sha256(), wire.sha256_hex(), wire.to_string());
    let _: Result<WireForm, HttpError> = ModifyOrderRequest::default()
        .wire(manja::kite::connect::models::OrderVariety::Regular, &id);
    let _: Result<WireForm, HttpError> = gtt.wire();
    let _: Result<WireForm, HttpError> = gtt.wire_modification(1);
    let _: Result<WireForm, HttpError> = HoldingsAuthorisationRequest::all().wire();
    let _: fn(&PositionConversionRequest) -> Result<WireForm, HttpError> =
        PositionConversionRequest::wire;
    let _: WireForm =
        WireForm::cancel_order(manja::kite::connect::models::OrderVariety::Regular, &id);
    let _: WireForm = WireForm::delete_trigger(1);
    let _: fn(&ApiKey, &RequestToken, &ApiSecret) -> WireForm = WireForm::session_exchange;
    let _: fn(&ApiKey, &AccessToken) -> WireForm = WireForm::session_invalidation;
    // Priority lanes: a marked client, and the lane on an admission grant.
    let control: HTTPClient = c.with_priority(Priority::Control);
    let _: Result<Response<Vec<Order>>, ManjaError> = control.orders().list_orders().await;
    let _: Result<AdmissionLimits, manja::kite::connect::admission::QuotaError> =
        AdmissionLimits::default().with_control_reserve(RateClass::Standard, 2);
    let _: fn(&AdmissionLimits, RateClass) -> u32 = AdmissionLimits::control_reserve;
    if let Ok(grant) = c
        .admission()
        .acquire_with_priority(
            Priority::Control,
            RateClass::Standard,
            None,
            std::time::Duration::ZERO,
        )
        .await
    {
        let _: Priority = grant.priority();
        let _: AdmissionError = AdmissionError::NoStandardCapacity;
        grant.consume();
    }
    // Synchronous admission and the refusal facts an error carries.
    if let Err(e) = c
        .admission()
        .try_acquire(Priority::Standard, RateClass::OrderPlacement, None)
    {
        let _: (
            manja::kite::obs::schema::AdmissionCause,
            Option<std::time::Duration>,
            Option<chrono::DateTime<chrono::FixedOffset>>,
        ) = (e.cause(), e.retry_after(), e.resets_at());
        let _ = matches!(
            e,
            AdmissionError::Refused { .. }
                | AdmissionError::DailyCeilingReached { .. }
                | AdmissionError::ModificationCeiling
        );
    }
    let _: fn(&HttpError) -> Option<std::time::Duration> = HttpError::retry_after;
    let _: fn(&HttpError) -> Option<chrono::DateTime<chrono::FixedOffset>> = HttpError::resets_at;
    let _: fn(&HttpError) -> Option<manja::kite::obs::schema::AdmissionCause> =
        HttpError::admission_cause;
    // Source identity and ingress stamps on responses and errors.
    let _: &SourceIdentity = c.source_identity();
    if let Ok(book) = c.orders().list_orders().await {
        let _: (SourceIdentity, IngressStamp, IngressStamp) =
            (book.meta.source, book.meta.sent_at, book.meta.received_at);
        let _: (
            manja::kite::envelope::ReceiveTime,
            manja::kite::envelope::MonotonicElapsed,
        ) = (book.meta.sent_at.wall, book.meta.sent_at.monotonic);
    }
    let _: fn(&HttpError) -> Option<&SourceIdentity> = HttpError::source;
    let _: fn(&HttpError) -> Option<u64> = HttpError::operation_id;
    let _: fn(&HttpError) -> Option<IngressStamp> = HttpError::sent_at;
    let _: fn(&HttpError) -> Option<IngressStamp> = HttpError::received_at;
    let _: HttpClientBuilder =
        HTTPClient::builder(Config::default()).identity(SourceIdentity::generate());
    // Dispatch: a mutation is a plain function returning a future the
    // client runs to completion; its outcome reaches the stream or the ring.
    let dispatched: Dispatched<OrderReceipt> = c.orders().place_order(&place);
    let _: (
        u64,
        Option<&HttpError>,
        Option<[u8; 32]>,
        Option<&str>,
        bool,
    ) = (
        dispatched.operation_id(),
        dispatched.refusal(),
        dispatched.wire_sha256(),
        dispatched.correlation(),
        dispatched.is_in_flight(),
    );
    let _ = format!("{dispatched:?}");
    let _: Result<Response<OrderReceipt>, HttpError> = dispatched.await;
    let _: Result<HTTPClient, ManjaError> = c.with_correlation("attempt-1");
    let _: Result<(), CloseError> = c.close(std::time::Duration::ZERO).await;
    let _: fn(&CloseError) -> usize = |e| e.in_flight;
    let _: usize = manja::kite::connect::dispatch::ORPHANED_OUTCOMES;
    let _: usize = manja::kite::connect::dispatch::MAX_CORRELATION_BYTES;
    let _: ReceiptIds = ReceiptIds::none(&());
    let _: (Option<OrderId>, Option<u64>) = (ReceiptIds::none(&()).order_id, None);
    let _: fn(&OrderReceipt) -> ReceiptIds = ReceiptIds::of_order;
    let _: fn(&GttReceipt) -> ReceiptIds = ReceiptIds::of_trigger;
    let built: Result<(HTTPClient, OutcomeStream), ManjaError> = HTTPClient::builder(
        Config::default().with_limits(HttpLimits::default().with_outcome_capacity(16).unwrap()),
    )
    .build_with_outcomes();
    if let Ok((client, mut stream)) = built {
        let _: usize = client.diagnostics().dispatches_in_flight;
        let _: Vec<Outcome> = client.diagnostics().orphaned_outcomes;
        let _: (u64, u64) = (
            client.diagnostics().orphaned_outcome_count,
            client.diagnostics().outcome_stream_dropped,
        );
        let _: (usize, bool) = (stream.len(), stream.is_empty());
        if let Some(Ok(o)) = stream.recv().await {
            let _: (u64, Option<String>, Method, Endpoint, [u8; 32]) = (
                o.operation_id,
                o.correlation,
                o.method,
                o.endpoint,
                o.wire_sha256,
            );
            let _: (TransportStage, HttpOperationResult, Option<HttpErrorKind>) =
                (o.stage, o.result, o.error);
            let _: (Option<u16>, u32, ReceiptIds, SourceIdentity) =
                (o.http_status, o.attempt, o.receipt, o.source);
            let _: (Option<IngressStamp>, Option<IngressStamp>) = (o.sent_at, o.received_at);
        }
        let _ = matches!(
            OutcomeStreamError::Overflow { dropped: 1 },
            OutcomeStreamError::Overflow { .. }
        );
    }
    // Headroom is a plain, copyable snapshot; both readers take the checked ID.
    let room: Headroom = c.admission().headroom(RateClass::OrderPlacement);
    let copy = room;
    let _: (u32, u32, Option<std::time::Duration>, Option<u32>) = (
        room.standard_now,
        room.control_now,
        room.next_slot_in,
        room.day_remaining,
    );
    let _ = (format!("{copy:?}"), copy == room);
    fn assert_copy<T: Copy>() {}
    assert_copy::<Headroom>();
    let _: u32 = c.admission().modifications_remaining(&id);
    let _: Result<Revision, CommandError> =
        h.subscribe([InstrumentToken::new(1)], Mode::Quote).await;
    let _: TickerStatus = h.status();
    let _: Result<(), TickerError> = h.shutdown().await;
}

// The session expiry rule is a protocol constant and a pure function, with no
// feature gate.
fn session_expiry() {
    let _: chrono::NaiveTime = SESSION_EXPIRY_IST;
    let issued: chrono::DateTime<chrono::FixedOffset> = "2026-10-12T01:00:00Z".parse().unwrap();
    let _: chrono::DateTime<chrono::FixedOffset> = session_expires_at(issued);
}

fn ticker_parts() -> Result<(TickerHandle, TickerEvents, TaskGuard), TickerSpawnError> {
    TickerBuilder::new(Credentials::new("k", "t").unwrap())
        .limits(TickerLimits::default().with_reconnect(ReconnectLimits::default()))
        .observability(Observability::disabled())
        .spawn()
}

#[test]
fn the_public_surface_is_reachable_and_constructible_offline() {
    let client = HTTPClient::builder(
        Config::default()
            .with_limits(HttpLimits::default().with_scheduler(SchedulerLimits::default())),
    )
    .admission(Admission::new(
        QuotaProfile::kite_v3(),
        AdmissionLimits::default(),
    ))
    .observability(Observability::disabled())
    .build()
    .unwrap();
    resources(&client, ApiKey::new("k").unwrap());
    old_call_shapes(client.clone());
    session_expiry();
    // Outside a runtime, a ticker cannot start: an explicit error.
    assert_eq!(ticker_parts().unwrap_err(), TickerSpawnError::NoRuntime);
    assert_eq!(
        TickerRequest::set_mode(vec![1], Mode::LTP).to_string(),
        r#"{"a":"mode","v":["ltp",[1]]}"#
    );
}

#[test]
fn status_carries_no_runtime_handle_and_events_are_serializable_data() {
    // Portable envelopes serialize; they hold no task or runtime field.
    let mut s = manja::kite::envelope::SourceSequencer::new(SourceIdentity::generate());
    s.begin_epoch();
    let o = RawObservation::new(
        s.next_key(),
        manja::kite::envelope::PayloadKind::Binary,
        manja::kite::envelope::ReceiveTime::from_unix_nanos(0),
        manja::kite::envelope::MonotonicElapsed::from_nanos(0),
        vec![1],
        1 << 20,
    )
    .unwrap();
    let json = serde_json::to_string(&o).unwrap();
    for field in ["task", "handle", "runtime", "join"] {
        assert!(!json.contains(field), "{json}");
    }
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TickerHandle>();
    assert_send_sync::<TickerStatus>();
}

/// The 0.2 dispatch permit API is gone from the HTTP slice: no public item
/// in `src/kite/connect` carries a retired name. The needles are assembled
/// from fragments so that this file itself does not spell them.
#[test]
fn the_dispatch_permit_api_is_absent_from_the_http_slice() {
    fn sources(dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/kite/connect");
    let mut files = Vec::new();
    sources(&root, &mut files);
    assert!(files.len() > 10, "the HTTP slice was scanned");
    let retired = [
        ["Dispatch", "Permit"].concat(),
        ["Permit", "Target"].concat(),
        ["pub async fn ", "admit("].concat(),
        ["_with_", "permit"].concat(),
        ["permit_", "validity"].concat(),
        ["B-HTTP-", "12"].concat(),
    ];
    for source in &files {
        for needle in &retired {
            assert!(
                !source.contains(needle.as_str()),
                "retired name present: {needle}"
            );
        }
    }
}
