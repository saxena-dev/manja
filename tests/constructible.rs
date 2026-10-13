//! Downstream-style construction of every public response type, receipt,
//! outcome record and checked type (`contract.md` §2.19): what a broker
//! double or a fixture-based test outside the crate has to be able to build.
//! Only public paths are used; nothing from manja's own test helpers. The
//! target declares no required features, so it runs on every feature row:
//! the models, errors, protocol and envelope types are always available, and
//! the outcome record, which needs the `http` feature, is gated inside.
//!
//! The inventory check works like this: `built!` evaluates a literal or a
//! constructor for a type and records the type's name as it does so, so the
//! list of constructed names is produced by the constructions themselves.
//! The test then scans every `pub struct` and `pub enum` in
//! `src/kite/connect/models/*.rs` and requires each name to appear in that
//! list, and it requires the names of the types that live elsewhere
//! (`kite::error`, `kite::connect::dispatch`, `kite::protocol`,
//! `kite::envelope`) by a fixed list. A new public model type without a
//! construction here fails the test.

mod downstream {
    use std::collections::{BTreeMap, HashMap};
    use std::time::Duration;

    use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone};
    #[cfg(feature = "http")]
    use manja::kite::connect::dispatch::{Outcome, ReceiptIds};
    use manja::kite::connect::models::{
        Auction, Available, BasketMargin, Candle, CandleInterval, Charges, Depth, DepthLevel,
        DividendType, Exchange, FullQuote, GST, GttCondition, GttOrder, GttOrderOutcome,
        GttOrderRequest, GttOrderResult, GttReceipt, GttRequest, GttStatus, GttTrigger, GttType,
        HistoricalData, HistoricalRequest, Holding, HoldingMtf, HoldingsAuthorisation,
        HoldingsAuthorisationRequest, IngressStamp, Instrument, InstrumentType, LTPQuote,
        MfHolding, MfInstrument, MfOrder, MfOrderStatus, MfOrderVariety, MfPlan, MfPurchaseType,
        MfSip, ModifyOrderRequest, OHLC, OHLCQuote, Order, OrderCharges, OrderChargesRequest,
        OrderMargin, OrderMarginRequest, OrderReceipt, OrderStatus, OrderType, OrderUpdate,
        OrderValidity, OrderVariety, PNL, PlaceOrderRequest, Position, PositionConversionRequest,
        PositionType, Positions, ProductType, ProfileMeta, QuoteMode, Quotes, RequestError,
        Response, ResponseMeta, Row, RowError, Rows, SchemeType, Segment, SegmentKind, SessionMeta,
        SipFrequency, SipStatus, SliceError, SliceResult, Tag, Trade, TransactionType,
        UnknownExchange, UserMargins, UserProfile, UserSession, Utilised,
    };
    use manja::kite::envelope::{MonotonicElapsed, ReceiveTime, RunId, SourceIdentity};
    use manja::kite::error::{
        BrokerError, HttpError, HttpErrorKind, KiteApiException, RetryInfo, TransportStage,
    };
    #[cfg(feature = "http")]
    use manja::kite::obs::schema::HttpOperationResult;
    use manja::kite::obs::schema::{AdmissionCause, Endpoint, Method};
    use manja::kite::protocol::{
        BrokerTimestamp, Inbound, InstrumentToken, MfOrderId, OrderId, PostbackChecksum, Quantity,
        ScaledPrice,
    };
    use manja::secrecy::Secret;

    /// Build `$value` as `$ty` and record the type's name.
    macro_rules! built {
        ($names:ident, $ty:ident, $value:expr) => {{
            let value: $ty = $value;
            $names.push(stringify!($ty));
            value
        }};
        ($names:ident, $ty:ty = $name:literal, $value:expr) => {{
            let value: $ty = $value;
            $names.push($name);
            value
        }};
    }

    fn ts() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(19_800)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 12, 9, 15, 0)
            .unwrap()
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()
    }

    fn stamp() -> IngressStamp {
        IngressStamp {
            wall: ReceiveTime::from_unix_nanos(1_760_000_000_000_000_000),
            monotonic: MonotonicElapsed::from_nanos(12_345),
        }
    }

    fn order_id() -> OrderId {
        OrderId::new("151220000000000").unwrap()
    }

    /// Every construction, in one place; returns the names recorded.
    #[expect(
        clippy::too_many_lines,
        reason = "one value of every public type, by design"
    )]
    pub fn build_everything() -> Vec<&'static str> {
        let mut names: Vec<&'static str> = Vec::new();
        let n = &mut names;

        // ---- checked types and wire values --------------------------------
        let id = built!(n, OrderId, order_id());
        let _ = built!(
            n,
            MfOrderId,
            MfOrderId::new("271b9e7e-2b4c-4b07-a3a9-1d7a0e6f4c11").unwrap()
        );
        let tag = built!(n, Tag, Tag::new("rebalance42").unwrap());
        let qty = built!(n, Quantity, Quantity::new(1).unwrap());
        let token = built!(n, InstrumentToken, InstrumentToken::new(408_065));
        let _ = built!(n, ScaledPrice, ScaledPrice::new(150_050, 100).unwrap());
        let broker_ts = built!(
            n,
            BrokerTimestamp,
            BrokerTimestamp::parse("2026-10-12 09:15:00").unwrap()
        );
        let _ = built!(n, PostbackChecksum, PostbackChecksum::new("abc"));
        let known = built!(n, Inbound<OrderStatus> = "Inbound", Inbound::Known(OrderStatus::Open));
        let unknown: Inbound<Exchange> = Inbound::from_wire("NOT_AN_EXCHANGE");
        assert!(matches!(unknown, Inbound::Unknown(_)));
        let _ = built!(
            n,
            UnknownExchange,
            match Inbound::<Exchange>::from_wire("NOT_AN_EXCHANGE") {
                Inbound::Unknown(value) => UnknownExchange(value),
                Inbound::Known(_) => unreachable!("not a known exchange"),
            }
        );
        let run = built!(n, RunId, RunId::from_u128(7));
        let source = built!(
            n,
            SourceIdentity,
            SourceIdentity::new("paper", run, "sim").unwrap()
        );
        let _ = built!(n, ReceiveTime, ReceiveTime::from_unix_nanos(0));
        let _ = built!(n, MonotonicElapsed, MonotonicElapsed::from_nanos(0));
        let stamp = built!(n, IngressStamp, stamp());

        // ---- enums ---------------------------------------------------------
        let exchange = built!(n, Exchange, Exchange::NSE);
        let _ = built!(n, GttType, GttType::Single);
        let _ = built!(n, GttStatus, GttStatus::Active);
        let interval = built!(n, CandleInterval, CandleInterval::Minute);
        let _ = built!(n, InstrumentType, InstrumentType::Equity);
        let _ = built!(n, QuoteMode, QuoteMode::Full);
        let _ = built!(n, MfOrderStatus, MfOrderStatus::Open);
        let _ = built!(n, MfOrderVariety, MfOrderVariety::Regular);
        let _ = built!(n, MfPurchaseType, MfPurchaseType::Fresh);
        let _ = built!(n, SipStatus, SipStatus::Active);
        let _ = built!(n, SipFrequency, SipFrequency::Monthly);
        let _ = built!(n, DividendType, DividendType::Growth);
        let _ = built!(n, SchemeType, SchemeType::Equity);
        let _ = built!(n, MfPlan, MfPlan::Direct);
        let variety = built!(n, OrderVariety, OrderVariety::Regular);
        let _ = built!(n, OrderStatus, OrderStatus::Open);
        let order_type = built!(n, OrderType, OrderType::Market);
        let product = built!(n, ProductType, ProductType::CashAndCarry);
        let validity = built!(n, OrderValidity, OrderValidity::Day);
        let side = built!(n, TransactionType, TransactionType::BUY);
        let position_type = built!(n, PositionType, PositionType::Day);
        let _ = built!(n, SegmentKind, SegmentKind::Equity);

        // ---- receipts and the order book ----------------------------------
        let slice_error = built!(
            n,
            SliceError,
            SliceError::new(
                Some(400),
                Some(Inbound::Known(KiteApiException::InputException)),
                Some("rejected".into()),
            )
        );
        let placed = built!(
            n,
            SliceResult,
            SliceResult::Placed {
                order_id: id.clone()
            }
        );
        let receipt = built!(
            n,
            OrderReceipt,
            OrderReceipt {
                order_id: id.clone(),
                slices: vec![placed, SliceResult::Failed(slice_error)],
            }
        );
        let order = built!(
            n,
            Order,
            Order {
                order_id: id.clone(),
                parent_order_id: None,
                exchange_order_id: Some("1100000000000000".into()),
                modified: Some(false),
                placed_by: "AB1234".into(),
                variety: Inbound::Known(variety),
                status: known,
                tradingsymbol: "INFY".into(),
                exchange: Inbound::Known(exchange),
                instrument_token: token,
                transaction_type: Inbound::Known(side),
                order_type: Inbound::Known(order_type),
                product: Inbound::Known(product),
                validity: Inbound::Known(validity),
                validity_ttl: None,
                price: 0.0,
                quantity: 1,
                trigger_price: 0.0,
                average_price: 0.0,
                pending_quantity: 1,
                filled_quantity: 0,
                disclosed_quantity: 0,
                cancelled_quantity: 0,
                market_protection: None,
                order_timestamp: Some(ts()),
                exchange_timestamp: None,
                exchange_update_timestamp: None,
                status_message: None,
                status_message_raw: None,
                auction_number: None,
                meta: None,
                tag: Some(tag.as_str().to_owned()),
                tags: None,
                guid: None,
                iceberg_legs: None,
                iceberg_quantity: None,
            }
        );
        let _ = built!(
            n,
            Trade,
            Trade {
                trade_id: "10000000".into(),
                order_id: id.clone(),
                exchange_order_id: None,
                tradingsymbol: "INFY".into(),
                exchange: Inbound::Known(exchange),
                instrument_token: token,
                transaction_type: Inbound::Known(side),
                product: Inbound::Known(product),
                average_price: 1500.5,
                quantity: 1,
                fill_timestamp: Some(ts()),
                order_timestamp: Some(broker_ts),
                exchange_timestamp: None,
            }
        );
        let mut update = built!(n, OrderUpdate, OrderUpdate::new(id.clone()));
        update.status = Some(Inbound::Known(OrderStatus::Open));
        update.tradingsymbol = Some("INFY".into());
        update.instrument_token = Some(token);
        update.exchange = Some(Inbound::Known(exchange));
        update.quantity = Some(1);
        update.filled_quantity = Some(0);
        update.pending_quantity = Some(1);
        update.cancelled_quantity = Some(0);
        update.checksum = Some(PostbackChecksum::new("abc"));
        assert_eq!(update.order_id, id);
        let row_error = built!(
            n,
            RowError,
            RowError {
                index: 1,
                error: "data error".into(),
                raw: serde_json::json!({ "order_id": "not an id" }),
            }
        );
        let row = built!(n, Row<Order> = "Row", Row::Decoded(order.clone()));
        let _ = built!(n, Rows<Order> = "Rows", Rows { rows: vec![row, Row::Rejected(row_error)] });

        // ---- the response envelope ----------------------------------------
        let meta = built!(
            n,
            ResponseMeta,
            ResponseMeta::new(None, 200, 7, 1, source.clone(), stamp, stamp,)
        );
        let _ = built!(n, Response<Order> = "Response", Response { data: order.clone(), meta });

        // ---- requests -----------------------------------------------------
        let _ = built!(
            n,
            RequestError,
            RequestError::new("quantity", "must be positive")
        );
        let _ = built!(
            n,
            PlaceOrderRequest,
            PlaceOrderRequest::new(
                variety,
                exchange,
                "INFY",
                side,
                order_type,
                qty,
                product,
                tag.clone(),
            )
        );
        let _ = built!(
            n,
            ModifyOrderRequest,
            ModifyOrderRequest {
                order_type: None,
                quantity: Some(qty),
                price: None,
                trigger_price: None,
                disclosed_quantity: None,
                validity: None,
                validity_ttl: None,
                market_protection: None,
            }
        );
        let _ = built!(
            n,
            PositionConversionRequest,
            PositionConversionRequest {
                tradingsymbol: "INFY".into(),
                exchange,
                transaction_type: side,
                position_type,
                quantity: qty,
                old_product: ProductType::CashAndCarry,
                new_product: ProductType::MarginIntradaySquareoff,
            }
        );
        let _ = built!(
            n,
            HoldingsAuthorisationRequest,
            HoldingsAuthorisationRequest {
                instruments: vec![("INE009A01021".into(), qty)],
            }
        );
        let gtt_order_request = built!(
            n,
            GttOrderRequest,
            GttOrderRequest {
                transaction_type: side,
                quantity: qty,
                order_type: OrderType::Limit,
                product,
                price: 702.5,
            }
        );
        let _ = built!(
            n,
            GttRequest,
            GttRequest {
                trigger_type: GttType::Single,
                exchange,
                tradingsymbol: "INFY".into(),
                trigger_values: vec![702.0],
                last_price: 798.0,
                orders: vec![gtt_order_request],
            }
        );
        let _ = built!(
            n,
            HistoricalRequest,
            HistoricalRequest {
                instrument_token: token,
                interval,
                from: ts(),
                to: ts(),
                continuous: false,
                oi: false,
            }
        );
        let _ = built!(
            n,
            OrderMarginRequest,
            OrderMarginRequest {
                exchange,
                tradingsymbol: "INFY".into(),
                transaction_type: side,
                variety,
                product,
                order_type,
                quantity: qty,
                price: 0.0,
                trigger_price: 0.0,
            }
        );
        let _ = built!(
            n,
            OrderChargesRequest,
            OrderChargesRequest {
                order_id: "151220000000000".into(),
                exchange,
                tradingsymbol: "INFY".into(),
                transaction_type: side,
                variety,
                product,
                order_type,
                quantity: qty,
                average_price: 1500.5,
            }
        );

        // ---- user and session ---------------------------------------------
        let profile_meta = built!(
            n,
            ProfileMeta,
            ProfileMeta {
                demat_consent: "consent".into()
            }
        );
        let _ = built!(
            n,
            UserProfile,
            UserProfile {
                user_id: "AB1234".into(),
                user_type: "individual".into(),
                email: "x@example.com".into(),
                user_name: "X".into(),
                user_shortname: "X".into(),
                broker: "ZERODHA".into(),
                exchanges: vec!["NSE".into()],
                products: vec!["CNC".into()],
                order_types: vec!["MARKET".into()],
                avatar_url: None,
                meta: profile_meta,
            }
        );
        let available = built!(
            n,
            Available,
            Available {
                cash: 0.0,
                opening_balance: 0.0,
                live_balance: 0.0,
                intraday_payin: 0.0,
                adhoc_margin: 0.0,
                collateral: 0.0,
            }
        );
        let utilised = built!(
            n,
            Utilised,
            Utilised {
                debits: 0.0,
                exposure: 0.0,
                m2m_realised: 0.0,
                m2m_unrealised: 0.0,
                option_premium: 0.0,
                payout: 0.0,
                span: 0.0,
                holding_sales: 0.0,
                turnover: 0.0,
                liquid_collateral: 0.0,
                stock_collateral: 0.0,
                delivery: 0.0,
            }
        );
        let segment = built!(
            n,
            Segment,
            Segment {
                enabled: true,
                net: 0.0,
                available,
                utilised
            }
        );
        let _ = built!(
            n,
            UserMargins,
            UserMargins {
                equity: Some(segment),
                commodity: None
            }
        );
        let session_meta = built!(
            n,
            SessionMeta,
            SessionMeta {
                demat_consent: String::new()
            }
        );
        let _ = built!(
            n,
            UserSession,
            UserSession {
                user_type: "individual".into(),
                email: "x@example.com".into(),
                user_name: "X".into(),
                user_shortname: "X".into(),
                broker: "ZERODHA".into(),
                exchanges: vec![],
                products: vec![],
                order_types: vec![],
                avatar_url: None,
                user_id: "AB1234".into(),
                api_key: Secret::new("key".into()),
                access_token: Secret::new("token".into()),
                public_token: Secret::new("public".into()),
                refresh_token: None,
                enctoken: None,
                login_time: Some(ts()),
                meta: Some(session_meta),
            }
        );

        // ---- portfolio ----------------------------------------------------
        let mtf = built!(
            n,
            HoldingMtf,
            HoldingMtf {
                quantity: 0,
                used_quantity: 0,
                average_price: 0.0,
                value: 0.0,
                initial_margin: 0.0,
            }
        );
        let _ = built!(
            n,
            Holding,
            Holding {
                tradingsymbol: "INFY".into(),
                exchange: Inbound::Known(exchange),
                instrument_token: token,
                isin: "INE009A01021".into(),
                product: Inbound::Known(product),
                price: 0.0,
                quantity: 1,
                used_quantity: 0,
                t1_quantity: 0,
                realised_quantity: 1,
                authorised_quantity: 0,
                authorised_date: None,
                authorisation: None,
                opening_quantity: 1,
                short_quantity: None,
                collateral_quantity: 0,
                collateral_type: None,
                discrepancy: false,
                average_price: 1400.0,
                last_price: 1500.0,
                close_price: 1490.0,
                pnl: 100.0,
                day_change: 10.0,
                day_change_percentage: 0.67,
                mtf: Some(mtf),
            }
        );
        let _ = built!(
            n,
            Auction,
            Auction {
                tradingsymbol: "INFY".into(),
                exchange: Inbound::Known(exchange),
                instrument_token: token,
                isin: "INE009A01021".into(),
                product: Inbound::Known(product),
                price: 0.0,
                quantity: 1,
                t1_quantity: 0,
                realised_quantity: 1,
                authorised_quantity: 0,
                authorised_date: None,
                opening_quantity: 1,
                collateral_quantity: 0,
                collateral_type: None,
                discrepancy: false,
                average_price: 1400.0,
                last_price: 1500.0,
                close_price: 1490.0,
                pnl: 100.0,
                day_change: 10.0,
                day_change_percentage: 0.67,
                auction_number: "20".into(),
            }
        );
        let position = built!(
            n,
            Position,
            Position {
                tradingsymbol: "INFY".into(),
                exchange: Inbound::Known(exchange),
                instrument_token: token,
                product: Inbound::Known(product),
                quantity: 1,
                overnight_quantity: 0,
                multiplier: 1,
                average_price: 1500.0,
                close_price: 0.0,
                last_price: 1500.0,
                value: -1500.0,
                pnl: 0.0,
                m2m: 0.0,
                unrealised: 0.0,
                realised: 0.0,
                buy_quantity: 1,
                buy_price: 1500.0,
                buy_value: 1500.0,
                buy_m2m: 1500.0,
                day_buy_quantity: 1,
                day_buy_price: 1500.0,
                day_buy_value: 1500.0,
                sell_quantity: 0,
                sell_price: 0.0,
                sell_value: 0.0,
                sell_m2m: 0.0,
                day_sell_quantity: 0,
                day_sell_price: 0.0,
                day_sell_value: 0.0,
            }
        );
        let _ = built!(
            n,
            Positions,
            Positions {
                net: vec![position.clone()],
                day: vec![position]
            }
        );
        let _ = built!(
            n,
            HoldingsAuthorisation,
            HoldingsAuthorisation {
                request_id: "na8QgCeQm05UHG6NL9sAGRiN".into()
            }
        );

        // ---- GTT ----------------------------------------------------------
        let gtt_receipt = built!(n, GttReceipt, GttReceipt { trigger_id: 123 });
        let outcome_row = built!(
            n,
            GttOrderOutcome,
            GttOrderOutcome {
                status: "success".into(),
                order_id: Some(id.clone()),
                rejection_reason: None,
            }
        );
        let gtt_result = built!(
            n,
            GttOrderResult,
            GttOrderResult {
                account_id: "AB1234".into(),
                exchange: Inbound::Known(exchange),
                tradingsymbol: "INFY".into(),
                validity: Inbound::Known(validity),
                product: Inbound::Known(product),
                order_type: Inbound::Known(OrderType::Limit),
                transaction_type: Inbound::Known(side),
                quantity: 1,
                price: 702.5,
                meta: None,
                timestamp: Some(ts()),
                triggered_at: 702.0,
                order_result: outcome_row,
            }
        );
        let gtt_order = built!(
            n,
            GttOrder,
            GttOrder {
                exchange: Inbound::Known(exchange),
                tradingsymbol: "INFY".into(),
                product: Inbound::Known(product),
                order_type: Inbound::Known(OrderType::Limit),
                transaction_type: Inbound::Known(side),
                quantity: 1,
                price: 702.5,
                result: Some(gtt_result),
            }
        );
        let condition = built!(
            n,
            GttCondition,
            GttCondition {
                exchange: Inbound::Known(exchange),
                tradingsymbol: "INFY".into(),
                trigger_values: vec![702.0],
                last_price: 798.0,
                instrument_token: Some(token),
            }
        );
        let _ = built!(
            n,
            GttTrigger,
            GttTrigger {
                id: gtt_receipt.trigger_id,
                user_id: "AB1234".into(),
                parent_trigger: None,
                trigger_type: Inbound::Known(GttType::Single),
                created_at: Some(ts()),
                updated_at: None,
                expires_at: None,
                status: Inbound::Known(GttStatus::Active),
                condition,
                orders: vec![gtt_order],
                meta: None,
            }
        );

        // ---- market -------------------------------------------------------
        let _ = built!(
            n,
            Instrument,
            Instrument {
                instrument_token: token,
                exchange_token: "1594".into(),
                tradingsymbol: "INFY".into(),
                name: Some("INFOSYS".into()),
                last_price: 0.0,
                expiry: None,
                strike: None,
                tick_size: 0.05,
                lot_size: 1,
                instrument_type: Inbound::Known(InstrumentType::Equity),
                segment: "NSE".into(),
                exchange: Inbound::Known(exchange),
            }
        );
        let ohlc = built!(
            n,
            OHLC,
            OHLC {
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5
            }
        );
        let level = built!(
            n,
            DepthLevel,
            DepthLevel {
                price: 1.0,
                orders: 1,
                quantity: 1
            }
        );
        let depth = built!(
            n,
            Depth,
            Depth {
                buy: vec![level.clone()],
                sell: vec![level]
            }
        );
        let _ = built!(
            n,
            FullQuote,
            FullQuote {
                instrument_token: token,
                timestamp: Some(ts()),
                last_trade_time: None,
                last_price: 1.5,
                volume: Some(10),
                average_price: None,
                buy_quantity: None,
                sell_quantity: None,
                open_interest: None,
                last_quantity: None,
                ohlc: ohlc.clone(),
                net_change: 0.0,
                lower_circuit_limit: None,
                upper_circuit_limit: None,
                oi: None,
                oi_day_high: None,
                oi_day_low: None,
                depth: Some(depth),
            }
        );
        let _ = built!(
            n,
            OHLCQuote,
            OHLCQuote {
                instrument_token: token,
                last_price: 1.5,
                ohlc
            }
        );
        let ltp = built!(
            n,
            LTPQuote,
            LTPQuote {
                instrument_token: token,
                last_price: 1.5
            }
        );
        let _ = built!(n, Quotes<LTPQuote> = "Quotes", Quotes {
            requested: vec!["NSE:INFY".into()],
            received: HashMap::from([("NSE:INFY".to_owned(), ltp)]),
            missing: vec![],
            unexpected: vec![],
        });
        let candle = built!(
            n,
            Candle,
            Candle {
                timestamp: ts(),
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5,
                volume: 10,
                oi: None,
            }
        );
        let _ = built!(
            n,
            HistoricalData,
            HistoricalData {
                candles: vec![candle]
            }
        );

        // ---- mutual funds -------------------------------------------------
        let _ = built!(
            n,
            MfOrder,
            MfOrder {
                order_id: MfOrderId::new("271b9e7e-2b4c-4b07-a3a9-1d7a0e6f4c11").unwrap(),
                exchange_order_id: None,
                tradingsymbol: "INF174K01LS2".into(),
                status: Some(Inbound::Known(MfOrderStatus::Open)),
                status_message: None,
                folio: None,
                fund: "Kotak Select Focus Fund - Direct Plan".into(),
                order_timestamp: Some(ts()),
                exchange_timestamp: Some(date()),
                settlement_id: None,
                transaction_type: Inbound::Known(side),
                amount: 5000.0,
                variety: Inbound::Known(MfOrderVariety::Regular),
                purchase_type: Some(Inbound::Known(MfPurchaseType::Fresh)),
                quantity: 0.0,
                price: None,
                last_price: 35.1,
                average_price: 0.0,
                placed_by: "AB1234".into(),
                last_price_date: Some(date()),
                tag: None,
            }
        );
        let _ = built!(
            n,
            MfSip,
            MfSip {
                sip_id: "1234".into(),
                tradingsymbol: "INF174K01LS2".into(),
                fund: "Kotak Select Focus Fund - Direct Plan".into(),
                dividend_type: Inbound::Known(DividendType::Growth),
                transaction_type: Inbound::Known(side),
                status: Inbound::Known(SipStatus::Active),
                created: Some(ts()),
                frequency: Inbound::Known(SipFrequency::Monthly),
                next_instalment: Some(date()),
                instalment_amount: 1000.0,
                instalments: -1,
                last_instalment: None,
                pending_instalments: -1,
                instalment_day: 1,
                completed_instalments: 0,
                tag: None,
                sip_reg_num: None,
                sip_type: None,
                trigger_price: None,
                step_up: Some(BTreeMap::new()),
            }
        );
        let _ = built!(
            n,
            MfHolding,
            MfHolding {
                folio: None,
                fund: "Kotak Select Focus Fund - Direct Plan".into(),
                tradingsymbol: "INF174K01LS2".into(),
                average_price: 30.0,
                last_price: 35.1,
                pnl: 5.1,
                last_price_date: Some(date()),
                quantity: 1.0,
                pledged_quantity: None,
            }
        );
        let _ = built!(
            n,
            MfInstrument,
            MfInstrument {
                tradingsymbol: "INF174K01LS2".into(),
                amc: "KotakMahindraMF".into(),
                name: "Kotak Select Focus Fund".into(),
                purchase_allowed: true,
                redemption_allowed: true,
                minimum_purchase_amount: 5000.0,
                purchase_amount_multiplier: 1.0,
                minimum_additional_purchase_amount: 1000.0,
                minimum_redemption_quantity: 0.001,
                redemption_quantity_multiplier: 0.001,
                dividend_type: Inbound::Known(DividendType::Growth),
                scheme_type: Inbound::Known(SchemeType::Equity),
                plan: Inbound::Known(MfPlan::Direct),
                settlement_type: "T3".into(),
                last_price: 35.1,
                last_price_date: Some(date()),
            }
        );

        // ---- margins and charges ------------------------------------------
        let pnl = built!(
            n,
            PNL,
            PNL {
                realised: 0.0,
                unrealised: 0.0
            }
        );
        let gst = built!(
            n,
            GST,
            GST {
                igst: 0.0,
                cgst: 0.0,
                sgst: 0.0,
                total: 0.0
            }
        );
        let charges = built!(
            n,
            Charges,
            Charges {
                transaction_tax: 0.0,
                transaction_tax_type: "stt".into(),
                exchange_turnover_charge: 0.0,
                sebi_turnover_charge: 0.0,
                brokerage: 0.0,
                stamp_duty: 0.0,
                gst,
                total: 0.0,
            }
        );
        let margin = built!(
            n,
            OrderMargin,
            OrderMargin {
                r#type: "equity".into(),
                tradingsymbol: "INFY".into(),
                exchange: Inbound::Known(exchange),
                span: 0.0,
                exposure: 0.0,
                option_premium: 0.0,
                additional: 0.0,
                bo: 0.0,
                cash: 0.0,
                var: 0.0,
                pnl,
                leverage: 1.0,
                charges: charges.clone(),
                total: 0.0,
            }
        );
        let _ = built!(
            n,
            BasketMargin,
            BasketMargin {
                initial: margin.clone(),
                r#final: margin.clone(),
                orders: vec![margin],
                charges: charges.clone(),
            }
        );
        let _ = built!(
            n,
            OrderCharges,
            OrderCharges {
                transaction_type: Inbound::Known(side),
                tradingsymbol: "INFY".into(),
                exchange: Inbound::Known(exchange),
                variety: Inbound::Known(variety),
                product: Inbound::Known(product),
                order_type: Inbound::Known(order_type),
                quantity: 1,
                price: 1500.5,
                charges,
            }
        );

        // ---- errors, one per stage ----------------------------------------
        let broker = built!(
            n,
            BrokerError,
            BrokerError::new(Some("InputException"), Some("bad input"))
        );
        let retry = built!(
            n,
            RetryInfo,
            RetryInfo {
                attempts: 1,
                max_attempts: 1
            }
        );
        let not_started = HttpError::new(
            HttpErrorKind::Validation,
            Method::Post,
            Endpoint::OrdersVariety,
            TransportStage::NotStarted,
        )
        .with_detail("quantity must be positive");
        let started = HttpError::new(
            HttpErrorKind::Transport,
            Method::Post,
            Endpoint::OrdersVariety,
            TransportStage::Started,
        )
        .with_attempt(1)
        .with_timeout()
        .with_sent_at(stamp)
        .with_operation(source.clone(), 7);
        let received = HttpError::new(
            HttpErrorKind::Broker,
            Method::Post,
            Endpoint::OrdersVariety,
            TransportStage::ResponseReceived,
        )
        .with_status(400)
        .with_broker(broker)
        .with_attempt(1)
        .with_retry(retry)
        .with_sent_at(stamp)
        .with_received_at(stamp);
        let unknown_stage = HttpError::new(
            HttpErrorKind::Transport,
            Method::Get,
            Endpoint::Orders,
            TransportStage::Unknown,
        )
        .with_source(std::io::Error::other("socket vanished"));
        let refused = HttpError::new(
            HttpErrorKind::Admission,
            Method::Post,
            Endpoint::OrdersVariety,
            TransportStage::NotStarted,
        )
        .with_admission(
            AdmissionCause::WindowFull,
            Some(Duration::from_secs(1)),
            None,
        );
        assert_eq!(not_started.stage(), TransportStage::NotStarted);
        assert!(!not_started.may_have_reached_broker());
        assert!(started.may_have_reached_broker() && started.is_timeout());
        assert_eq!(received.http_status(), Some(400));
        assert!(received.broker().is_some());
        assert_eq!(unknown_stage.stage(), TransportStage::Unknown);
        assert_eq!(refused.retry_after(), Some(Duration::from_secs(1)));
        let _ = built!(n, HttpError, not_started);

        // ---- the outcome record (http only: the dispatch module) ----------
        #[cfg(feature = "http")]
        {
            let receipt_ids = built!(n, ReceiptIds, ReceiptIds::of_order(&receipt));
            let mut outcome = built!(
                n,
                Outcome,
                Outcome::new(
                    7,
                    Method::Post,
                    Endpoint::OrdersVariety,
                    [0; 32],
                    TransportStage::ResponseReceived,
                    HttpOperationResult::Ok,
                    source,
                )
            );
            outcome.http_status = Some(200);
            outcome.attempt = 1;
            outcome.receipt = receipt_ids;
            outcome.sent_at = Some(stamp);
            outcome.received_at = Some(stamp);
            assert_eq!(outcome.receipt.order_id.as_ref(), Some(&id));
        }
        #[cfg(not(feature = "http"))]
        let _ = (receipt, source);

        names.sort_unstable();
        names.dedup();
        names
    }
}

/// Types outside `kite::connect::models` that the downstream module must
/// build: the error types in `kite::error` and the checked and wire types
/// in `kite::protocol` and `kite::envelope`. The outcome record's two types
/// in `kite::connect::dispatch` join the list when the `http` feature is on.
const CONSTRUCTED_ELSEWHERE: &[&str] = &[
    "BrokerError",
    "HttpError",
    "RetryInfo",
    "OrderId",
    "MfOrderId",
    "Quantity",
    "InstrumentToken",
    "ScaledPrice",
    "BrokerTimestamp",
    "PostbackChecksum",
    "OrderUpdate",
    "Inbound",
    "SourceIdentity",
    "RunId",
    "ReceiveTime",
    "MonotonicElapsed",
];

/// Every public type of `kite::connect::models` is constructed by the
/// downstream module, as are the types listed in `CONSTRUCTED_ELSEWHERE`.
/// The inventory comes from the source files, so a new `pub struct` or
/// `pub enum` in the models fails here until it is built below.
#[test]
fn every_public_response_type_is_constructible_downstream() {
    let built = downstream::build_everything();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/kite/connect/models");
    let mut inventory = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        for line in std::fs::read_to_string(&path).unwrap().lines() {
            let rest = line
                .strip_prefix("pub struct ")
                .or_else(|| line.strip_prefix("pub enum "));
            if let Some(rest) = rest {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                inventory.push(name);
            }
        }
    }
    inventory.sort_unstable();
    inventory.dedup();
    assert!(
        inventory.len() > 70,
        "the models were scanned: {inventory:?}"
    );
    let missing: Vec<&String> = inventory
        .iter()
        .filter(|name| !built.contains(&name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "public model types without a downstream construction: {missing:?}"
    );
    #[cfg(feature = "http")]
    let elsewhere: Vec<&str> = [CONSTRUCTED_ELSEWHERE, &["Outcome", "ReceiptIds"]].concat();
    #[cfg(not(feature = "http"))]
    let elsewhere: Vec<&str> = CONSTRUCTED_ELSEWHERE.to_vec();
    let missing: Vec<&&str> = elsewhere
        .iter()
        .filter(|name| !built.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "types without a downstream construction: {missing:?}"
    );
}
