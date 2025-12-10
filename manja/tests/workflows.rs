use std::collections::HashMap;

use tokio::join;

use manja::{
    BasketMargin, Exchange, KiteApiResponse, MfHolding, Order, OrderMarginRequest,
    OrderReceipt, OrderStatus, OrderType, OrderValidity, OrderVariety, ProductType,
    TransactionType,
};
use manja::workflows;

mod support;
use support::{
    add_mocks, make_manja_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
};

fn mock_map_place_order() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
    let mut mmap = HashMap::new();
    mmap.insert(
        ("POST", "/orders/regular"),
        "./kiteconnect-mocks/order_response.json",
    );
    mmap
}

fn mock_map_square_off() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
    let mut mmap = HashMap::new();
    mmap.insert(
        ("GET", "/portfolio/positions"),
        "./kiteconnect-mocks/positions.json",
    );
    mmap.insert(
        ("POST", "/orders/regular"),
        "./kiteconnect-mocks/order_response.json",
    );
    mmap
}

fn mock_map_margin_and_order() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
    let mut mmap = HashMap::new();
    mmap.insert(
        ("POST", "/margins/basket?consider_positions=true"),
        "./kiteconnect-mocks/basket_margins.json",
    );
    mmap.insert(
        ("POST", "/orders/regular"),
        "./kiteconnect-mocks/order_response.json",
    );
    mmap
}

fn mock_map_mf_holdings() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
    let mut mmap = HashMap::new();
    mmap.insert(
        ("GET", "/mf/holdings"),
        "./kiteconnect-mocks/mf_holdings.json",
    );
    mmap
}

fn sample_margin_request() -> OrderMarginRequest {
    OrderMarginRequest {
        exchange: Exchange::NSE,
        tradingsymbol: "INFY".to_string(),
        transaction_type: TransactionType::BUY,
        variety: OrderVariety::Regular,
        product: ProductType::CashAndCarry,
        order_type: OrderType::Market,
        quantity: 1,
        price: 0.0,
        trigger_price: 0.0,
    }
}

fn sample_order() -> Order {
    Order {
        order_id: String::new(),
        parent_order_id: None,
        exchange_order_id: None,
        modified: false,
        placed_by: String::new(),
        variety: OrderVariety::Regular,
        status: OrderStatus::Open,
        tradingsymbol: "INFY".to_string(),
        exchange: Exchange::NSE.to_string(),
        instrument_token: 0,
        transaction_type: TransactionType::BUY,
        order_type: OrderType::Market,
        product: ProductType::CashAndCarry,
        validity: OrderValidity::Day.to_string(),
        price: 0.0,
        quantity: 1,
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
        tag: Some("margin-check".to_string()),
        guid: String::new(),
        iceberg_legs: None,
        iceberg_quantity: None,
        validity_ttl: None,
        tags: None,
    }
}

#[tokio::test]
async fn workflows_place_cash_market_order_success() {
    let (server, mut client) = make_manja_test_client().await;
    let (_server,) = join!(add_mocks(server, mock_map_place_order()));

    let response: KiteApiResponse<OrderReceipt> = workflows::place_cash_market_order(
        &mut client,
        Exchange::NSE,
        "INFY".to_string(),
        1,
        TransactionType::BUY,
        ProductType::CashAndCarry,
        Some("test-order".to_string()),
    )
    .await
    .unwrap();

    let expected: OrderReceipt =
        read_to_object("./kiteconnect-mocks/order_response.json");

    assert_eq!(response.status, "success");
    assert_eq!(response.data.unwrap().order_id, expected.order_id);
}

#[tokio::test]
async fn workflows_square_off_position_by_symbol_success() {
    let (server, mut client) = make_manja_test_client().await;
    let (_server,) = join!(add_mocks(server, mock_map_square_off()));

    let receipts = workflows::square_off_position_by_symbol(
        &mut client,
        Exchange::MCX,
        "LEADMINI17DECFUT",
        Some(ProductType::Normal),
        Some("square-off".to_string()),
    )
    .await
    .unwrap();

    assert_eq!(receipts.len(), 1);

    let expected: OrderReceipt =
        read_to_object("./kiteconnect-mocks/order_response.json");

    assert_eq!(receipts[0].status, "success");
    assert_eq!(
        receipts[0].data.as_ref().unwrap().order_id,
        expected.order_id
    );
}

#[tokio::test]
async fn workflows_place_order_with_margin_check_success() {
    let (server, mut client) = make_manja_test_client().await;
    let (_server,) = join!(add_mocks(server, mock_map_margin_and_order()));

    let margin_request = sample_margin_request();
    let order = sample_order();

    let (margin_resp, order_resp) =
        workflows::place_order_with_margin_check(&mut client, margin_request, order)
            .await
            .unwrap();

    let expected_margin: BasketMargin =
        read_to_object("./kiteconnect-mocks/basket_margins.json");
    let expected_order: OrderReceipt =
        read_to_object("./kiteconnect-mocks/order_response.json");

    assert_eq!(margin_resp.status, "success");
    assert_eq!(
        margin_resp.data.as_ref().unwrap().initial.total,
        expected_margin.initial.total
    );

    assert_eq!(order_resp.status, "success");
    assert_eq!(
        order_resp.data.as_ref().unwrap().order_id,
        expected_order.order_id
    );
}

#[tokio::test]
async fn workflows_mf_holdings_by_tradingsymbol_success() {
    let (server, mut client) = make_manja_test_client().await;
    let (_server,) = join!(add_mocks(server, mock_map_mf_holdings()));

    let tradingsymbol = "INF761K01884";

    let holdings = workflows::mf_holdings_by_tradingsymbol(
        &mut client,
        tradingsymbol,
    )
    .await
    .unwrap();

    let expected: Vec<MfHolding> =
        read_to_object("./kiteconnect-mocks/mf_holdings.json");
    let expected_filtered: Vec<MfHolding> = expected
        .into_iter()
        .filter(|h| h.tradingsymbol == tradingsymbol)
        .collect();

    assert_eq!(holdings.len(), expected_filtered.len());
    assert!(holdings
        .iter()
        .all(|h| h.tradingsymbol == tradingsymbol));
}
