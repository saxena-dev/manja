//! High-level workflow helpers built on top of [`ManjaClient`].
//!
//! These helpers compose existing typed HTTP APIs (orders, portfolio, margins)
//! into small, ergonomic workflows while remaining thin wrappers. They do not
//! introduce additional business logic beyond orchestrating underlying calls.

use crate::{
    BasketMargin, Exchange, KiteApiResponse, ManjaClient, MfHolding, Order,
    OrderMarginRequest, OrderReceipt, Positions, ProductType, Result, TransactionType,
};
use crate::{OrderStatus, OrderType, OrderVariety};

/// Place a simple cash-equity market order using sensible defaults.
///
/// This helper constructs a minimal [`Order`] payload and delegates to
/// [`ManjaClient::orders`].
///
/// Defaults:
/// - `OrderVariety::Regular`
/// - `OrderType::Market`
/// - `OrderValidity::Day`
/// - `price = 0.0`, `trigger_price = 0.0`
///
/// All other fields on [`Order`] are populated with neutral defaults and are
/// ignored by the Kite HTTP API for placement.
pub async fn place_cash_market_order(
    client: &mut ManjaClient,
    exchange: Exchange,
    tradingsymbol: String,
    quantity: u32,
    transaction_type: TransactionType,
    product: ProductType, // typically `ProductType::CashAndCarry`
    tag: Option<String>,
) -> Result<KiteApiResponse<OrderReceipt>> {
    let order = build_simple_order(
        exchange,
        tradingsymbol,
        quantity,
        transaction_type,
        product,
        OrderType::Market,
        OrderVariety::Regular,
        tag,
    );

    client.orders().place_order(&order).await
}

/// Square off an open position for the given symbol and exchange.
///
/// This helper:
/// - Fetches positions using [`ManjaClient::portfolio`].
/// - Filters by `exchange`, `tradingsymbol`, and optional `product`.
/// - For each matching position with non-zero quantity, places a market order
///   on the opposite side to close it.
///
/// It returns one [`KiteApiResponse<OrderReceipt>`] per closing order placed.
pub async fn square_off_position_by_symbol(
    client: &mut ManjaClient,
    exchange: Exchange,
    tradingsymbol: &str,
    product: Option<ProductType>,
    tag: Option<String>,
) -> Result<Vec<KiteApiResponse<OrderReceipt>>> {
    let positions_resp = client.portfolio().get_positions().await?;
    let positions = positions_resp
        .data
        .unwrap_or(Positions {
            net: Vec::new(),
            day: Vec::new(),
        });

    let exchange_str = exchange.to_string();
    let product_str = product.as_ref().map(|p| p.to_string());

    let mut receipts = Vec::new();

    for position in positions.net.into_iter() {
        if position.tradingsymbol != tradingsymbol {
            continue;
        }
        if position.exchange != exchange_str {
            continue;
        }
        if let Some(ref expected) = product_str {
            if &position.product != expected {
                continue;
            }
        }

        let qty = position.quantity;
        if qty == 0 {
            continue;
        }

        let abs_qty = qty.unsigned_abs();

        let transaction_type = if qty > 0 {
            TransactionType::SELL
        } else {
            TransactionType::BUY
        };

        // If product was not supplied explicitly, attempt to derive it from
        // the position's product string. If we cannot, skip this position.
        let product_for_order = match product {
            Some(ref p) => p.clone(),
            None => match product_from_str(&position.product) {
                Some(p) => p,
                None => continue,
            },
        };

        let quantity_u32 = match u32::try_from(abs_qty) {
            Ok(q) if q > 0 => q,
            _ => {
                return Err("position quantity is too large to be represented as u32".into());
            }
        };

        let order = build_simple_order(
            exchange.clone(),
            position.tradingsymbol.clone(),
            quantity_u32,
            transaction_type,
            product_for_order,
            OrderType::Market,
            OrderVariety::Regular,
            tag.clone(),
        );

        let receipt = client.orders().place_order(&order).await?;
        receipts.push(receipt);
    }

    Ok(receipts)
}

/// Calculate margin for a single order and, if successful, place the order.
///
/// This helper:
/// - Calls [`ManjaClient::margins`] with the provided [`OrderMarginRequest`].
/// - If that call succeeds, places the provided [`Order`] via
///   [`ManjaClient::orders`].
///
/// It returns both the margin response and the order placement receipt. Any
/// error from either step is propagated via [`Result`].
pub async fn place_order_with_margin_check(
    client: &mut ManjaClient,
    margin_request: OrderMarginRequest,
    order: Order,
) -> Result<(KiteApiResponse<BasketMargin>, KiteApiResponse<OrderReceipt>)> {
    let margin_requests = [margin_request];
    let margin_response = client
        .margins()
        .basket(&margin_requests, true)
        .await?;
    let order_response = client.orders().place_order(&order).await?;
    Ok((margin_response, order_response))
}

/// Retrieve MF holdings filtered by `tradingsymbol`.
///
/// This helper:
/// - Calls [`ManjaClient::mutual_funds().holdings()`].
/// - Filters holdings whose `tradingsymbol` matches the provided value.
/// - Returns the filtered list directly, without wrapping it in
///   [`KiteApiResponse`].
pub async fn mf_holdings_by_tradingsymbol(
    client: &mut ManjaClient,
    tradingsymbol: &str,
) -> Result<Vec<MfHolding>> {
    let resp = client.mutual_funds().holdings().await?;
    let holdings = resp.data.unwrap_or_default();
    Ok(holdings
        .into_iter()
        .filter(|h| h.tradingsymbol == tradingsymbol)
        .collect())
}

fn build_simple_order(
    exchange: Exchange,
    tradingsymbol: String,
    quantity: u32,
    transaction_type: TransactionType,
    product: ProductType,
    order_type: OrderType,
    variety: OrderVariety,
    tag: Option<String>,
) -> Order {
    Order {
        order_id: String::new(),
        parent_order_id: None,
        exchange_order_id: None,
        modified: false,
        placed_by: String::new(),
        variety,
        status: OrderStatus::Open,
        tradingsymbol,
        exchange: exchange.to_string(),
        instrument_token: 0,
        transaction_type,
        order_type,
        product,
        validity: crate::OrderValidity::Day.to_string(),
        price: 0.0,
        quantity,
        trigger_price: 0.0,
        average_price: 0.0,
        pending_quantity: 0,
        filled_quantity: 0,
        disclosed_quantity: 0,
        order_timestamp: None,
        exchange_timestamp: None,
        exchange_update_timestamp: None,
        status_message: None,
        status_message_raw: None,
        cancelled_quantity: 0,
        auction_number: None,
        meta: serde_json::Value::Null,
        tag,
        guid: String::new(),
        iceberg_legs: None,
        iceberg_quantity: None,
        validity_ttl: None,
        tags: None,
    }
}

fn product_from_str(value: &str) -> Option<ProductType> {
    match value {
        "CNC" => Some(ProductType::CashAndCarry),
        "NRML" => Some(ProductType::Normal),
        "MIS" => Some(ProductType::MarginIntradaySquareoff),
        "MTF" => Some(ProductType::MarginTradingFacility),
        _ => None,
    }
}
