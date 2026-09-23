//! Downstream-style reachability: every public signature's
//! request, response, error, command, status, guard and bound types are
//! named here through their documented public paths. It compiles against
//! the default features and touches no network.

// The imports are the check: each must resolve through its public path.
#![allow(dead_code, deprecated, unused_imports)]

use manja::kite::connect::admission::{
    Admission, AdmissionError, AdmissionGrant, AdmissionLimits, QuotaProfile, RateClass, Window,
};
use manja::kite::connect::api::{Charges, Gtt, Margins, Market, Orders, Portfolio, Session, User};
use manja::kite::connect::client::{HTTPClient, HttpClientBuilder, HttpDiagnostics, HttpFailure};
use manja::kite::connect::config::{Config, HttpLimits};
use manja::kite::connect::credentials::{
    AccessToken, ApiKey, ApiSecret, CredentialError, Credentials, RequestToken,
};
use manja::kite::connect::models::{
    Candle, CandleInterval, Exchange, FullQuote, GttCondition, GttOrder, GttOrderOutcome,
    GttOrderRequest, GttOrderResult, GttReceipt, GttRequest, GttStatus, GttTrigger, GttType,
    HistoricalData, HistoricalRequest, Holding, Instrument, KiteApiResponse, LTPQuote,
    ModifyOrderRequest, OHLCQuote, Order, OrderReceipt, PlaceOrderRequest, Position,
    PositionConversionRequest, Positions, QuoteMode, Quotes, RequestError, Trade, UserSession,
};
use manja::kite::connect::scheduler::{DispatchPermit, PermitTarget, SchedulerLimits};
use manja::kite::decoder::adapter::{Adapter, AdapterError, Decoded, DecodedEvent, Versions};
use manja::kite::decoder::framing::{Frame, FramingError, FramingLimits, Message, PacketFamily};
use manja::kite::decoder::packets::{Depth, DepthEntry, Packet, PacketError};
use manja::kite::decoder::text::{TextError, TextEvent, TextLimits};
use manja::kite::envelope::{
    GapFacts, LifecycleEvent, LifecycleKind, Payload, RawObservation, SourceIdentity, SourceKey,
};
use manja::kite::error::{BrokerError, HttpError, HttpErrorKind, ManjaError, TransportStage};
use manja::kite::obs::{
    BridgeRecorder, InMemoryRecorder, Instrument as MetricInstrument, MetricRecorder, Observability,
};
use manja::kite::protocol::{Inbound, InstrumentToken, OrderUpdate, Quantity, ScaledPrice};
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
use manja::kite::ticker::{
    KiteStreamCredentials, Mode, StreamState, TickerRequest, TickerStream, WebSocketClient,
};

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
    let _: Session<'_> = c.session(key);
}

fn old_call_shapes(mut c: HTTPClient) {
    let m = &mut c;
    let _ = m.orders();
    let _ = m.market();
}

// Signatures of the main async operations, by type.
async fn signatures(c: &HTTPClient, h: &TickerHandle) {
    let _: Result<KiteApiResponse<Vec<Order>>, ManjaError> = c.orders().list_orders().await;
    let _: Result<KiteApiResponse<Vec<Holding>>, ManjaError> = c.portfolio().get_holdings().await;
    let _: Result<KiteApiResponse<Positions>, ManjaError> = c.portfolio().get_positions().await;
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
    let _: Result<KiteApiResponse<GttReceipt>, ManjaError> = c.gtt().place_trigger(&gtt).await;
    let _: Result<KiteApiResponse<GttReceipt>, ManjaError> = c.gtt().modify_trigger(1, &gtt).await;
    let _: Result<KiteApiResponse<GttReceipt>, ManjaError> = c.gtt().delete_trigger(1).await;
    let _: Result<KiteApiResponse<Vec<GttTrigger>>, ManjaError> = c.gtt().list_triggers().await;
    let _: Result<KiteApiResponse<GttTrigger>, ManjaError> = c.gtt().get_trigger(1).await;
    let _: Result<KiteApiResponse<Quotes<LTPQuote>>, ManjaError> =
        c.market().get_quotes::<LTPQuote>(&["NSE:INFY"]).await;
    let _: Result<Vec<Instrument>, ManjaError> = c.market().get_instruments_all().await;
    let at = chrono::DateTime::parse_from_rfc3339("2017-12-15T09:15:00+05:30").unwrap();
    let history = HistoricalRequest::new(InstrumentToken::new(5633), CandleInterval::Day, at, at);
    let _: Result<KiteApiResponse<HistoricalData>, ManjaError> =
        c.market().get_historical(&history).await;
    let _: fn(&HistoricalData) -> &Vec<Candle> = |d| &d.candles;
    let _: Result<DispatchPermit, ManjaError> = c.admit(PermitTarget::PlaceOrder).await;
    let _: HttpDiagnostics = c.diagnostics();
    let _: Result<Revision, CommandError> =
        h.subscribe([InstrumentToken::new(1)], Mode::Quote).await;
    let _: TickerStatus = h.status();
    let _: Result<(), TickerError> = h.shutdown().await;
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
    // Outside a runtime, a ticker cannot start: an explicit error.
    assert_eq!(ticker_parts().unwrap_err(), TickerSpawnError::NoRuntime);
    assert_eq!(
        TickerRequest::set_mode(vec![1], Mode::LTP).to_string(),
        TickerRequest::subscribe_with_mode(vec![1], Mode::LTP).to_string()
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
