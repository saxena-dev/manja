use std::collections::HashMap;

use tokio::join;

use manja::{
    BasketMargin, Exchange, OrderCharges, OrderChargesRequest, OrderMarginRequest,
    OrderType, OrderVariety, ProductType, TransactionType,
};

mod support;
use support::{
    add_mocks, make_manja_test_client, read_to_object, APIEndpoint, HTTPMethod, TestResponse,
};

fn mock_map() -> HashMap<(HTTPMethod, APIEndpoint), TestResponse> {
    let mut mmap = HashMap::new();
    mmap.insert(
        ("POST", "/margins/orders"),
        "./kiteconnect-mocks/order_margins.json",
    );
    mmap.insert(
        ("POST", "/margins/basket?consider_positions=true"),
        "./kiteconnect-mocks/basket_margins.json",
    );
    mmap.insert(
        ("POST", "/charges/orders"),
        "./kiteconnect-mocks/virtual_contract_note.json",
    );
    mmap
}

fn sample_order_margin_request() -> OrderMarginRequest {
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

fn sample_order_charges_requests() -> Vec<OrderChargesRequest> {
    vec![
        OrderChargesRequest {
            order_id: "111111111".to_string(),
            exchange: Exchange::NSE,
            tradingsymbol: "SBIN".to_string(),
            transaction_type: TransactionType::BUY,
            variety: OrderVariety::Regular,
            product: ProductType::CashAndCarry,
            order_type: OrderType::Market,
            quantity: 1,
            average_price: 560.0,
        },
        OrderChargesRequest {
            order_id: "2222222222".to_string(),
            exchange: Exchange::MCX,
            tradingsymbol: "GOLDPETAL24AUGFUT".to_string(),
            transaction_type: TransactionType::SELL,
            variety: OrderVariety::Regular,
            product: ProductType::Normal,
            order_type: OrderType::Limit,
            quantity: 1,
            average_price: 5862.0,
        },
    ]
}

#[tokio::test]
async fn facade_margins_basket_success() {
    let (server, mut client) = make_manja_test_client().await;
    let (_server,) = join!(add_mocks(server, mock_map()));

    let requests = vec![sample_order_margin_request()];
    let response = client
        .margins()
        .basket(&requests, true)
        .await
        .unwrap();

    let expected =
        read_to_object::<BasketMargin>("./kiteconnect-mocks/basket_margins.json");

    let basket = response.data.expect("expected basket margin data");
    assert_eq!(basket.initial.total, expected.initial.total);
    assert_eq!(basket.r#final.total, expected.r#final.total);
}

#[tokio::test]
async fn facade_charges_orders_success() {
    let (server, mut client) = make_manja_test_client().await;
    let (_server,) = join!(add_mocks(server, mock_map()));

    let requests = sample_order_charges_requests();
    let response = client
        .charges()
        .orders(&requests)
        .await
        .unwrap();

    let expected = read_to_object::<Vec<OrderCharges>>(
        "./kiteconnect-mocks/virtual_contract_note.json",
    );

    let charges = response.data.expect("expected order charges data");
    assert_eq!(charges.len(), expected.len());
    assert_eq!(charges[0].tradingsymbol, expected[0].tradingsymbol);
}
