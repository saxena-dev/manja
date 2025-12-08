manja
=====

> **Manja** (IPA: /maːŋdʒʱaː/) n.: A type of abrasive string utilized primarily for flying fighter kites, especially prevalent in South Asian countries. It is crafted by coating cotton string with powdered glass or a similar abrasive substance.

This crate provides a Rust client library for [Zerodha](https://zerodha.com/)'s [Kite Connect](https://kite.trade/) trading APIs (a set of REST-like HTTP APIs).

The primary entrypoint is the `ManjaClient` facade, which wraps the lower-level HTTP client and exposes typed API groups for Kite domains (user, session, orders, portfolio, market, margins, etc.).

## Quickstart

```rust ignore
use manja::ManjaClient;
use manja::{KiteApiResponse, UserProfile};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a client using environment-based configuration.
    // Required env vars depend on your Kite app configuration.
    let mut client = ManjaClient::from_env();

    // Assume you have obtained a request token via a browser-based login flow.
    let request_token = "<request_token>".to_string();

    // Login flow: create a user session
    let session = client.session();
    let kite_session = session.generate_session(&request_token).await?;

    // Fetch the user profile using the authenticated session
    let profile: KiteApiResponse<UserProfile> = client.user().profile().await?;
    println!("User: {:?}", profile.data);

    Ok(())
}
```

## `manja` Features

`manja` strives to improve the developer experience by providing better support in IDEs with features like auto-completion, type-inference, and inline documentation.

- [x] **Type safe**
    - *Compile-time Type Checking*: type safety ensures that errors related to type mismatches are caught during compilation rather than at runtime.
    - *Consistent Data Models*: `manja` uses strongly typed data models that match Kite Connect API's expected inputs and outputs.
    - *Enhanced Security*: by ensuring that only valid data types are sent to and received from the API, the risk of data-related vulnerabilities is reduced.
    - *Automatic Serialization/Deserialization*: `manja` handles the serialization (converting data structures to JSON) and deserialization (converting JSON responses back to data structures) automatically and correctly. This ensures that the data sent to and received from Kite Connect API adheres to the expected types.
    
- [x] **Asynchronous**: built on the performant `tokio` async-runtime, `manja` delivers unmatched performance, ensuring your applications run faster and more efficiently than ever before.
    - *Resource Efficiency*: maximize the use of your system's resources. `manja`'s asynchronous nature allows for optimal resource management, reducing overhead and improving overall performance.
    - *Concurrent Task Handling*: manage multiple tasks simultaneously without sacrificing performance or reliability.
    - *Improved latency*: experience reduced latency and faster response times, ensuring your applications are always responsive.

- [x] **Distributed Logging**: stay ahead of issues with real-time distributed logging using the `tracing` crate.
    - *Streamline Development*: facilitate smoother development cycles with better debugging and faster issue resolution.
    - *Reduce Downtime*: with real-time insights and quick access to logs, identify and resolve issues faster, minimizing downtime.
    - *Enhance User Experience*: quickly address errors and performance bottlenecks to provide a better experience for your users.

- [x] **WebSocket** support for streaming binary market data.
    - *Auto-reconnect Mechanism*: `manja` provides a reliable async WebSocket client with a configurable exponential backoff retry mechanism.
 
- [x] **WebDriver** integration for retrieving `request token` from the redirect URL after successfully authenticating with the Kite platform.



## Kite Connect 3.0 API: Supported Endpoints

- [x] **User**
    - [x] POST      `/session/token`                Authenticate and obtain the `access_token` after the login flow
    - [x] GET       `/user/profile`                 Retrieve the user profile
    - [x] GET       `/user/margins/:segment`        Retrieve detailed funds and margin information
    - [x] DELETE    `/session/token`                Logout and invalidate the API session and `access_token`
- [x] **Orders**
    - [x] POST      `/orders/:variety`              Place an order of a particular variety
    - [x] PUT       `/orders/:variety/:order_id`    Modify an open or pending order
    - [x] DELETE    `/orders/:variety/:order_id`    Cancel an open or pending order
    - [x] GET       `/orders`                       Retrieve the list of all orders (open and executed) for the day
    - [x] GET       `/orders/:order_id`             Retrieve the history of a given order
    - [x] GET       `/trades`                       Retrieve the list of all executed trades for the day
    - [x] GET       `/orders/:order_id/trades`      Retrieve the trades generated by an order
- [ ] **GTT - Good Till Triggered orders**
    - [ ] POST      `/gtt/triggers`                 Places a GTT
    - [ ] GET       `/gtt/triggers`                 Retrieve a list of all GTTs visible in GTT order book
    - [ ] GET       `/gtt/triggers/:id`             Retrieve an individual trigger
    - [ ] PUT       `/gtt/triggers/:id`             Modify an active GTT
    - [ ] DELETE    `/gtt/triggres/:id`             Delete an active GTT
- [x] **Portfolio**
    - [x] GET       `/portfolio/holdings`           Retrieve the list of long term equity holdings
    - [x] GET       `/portfolio/positions`          Retrieve the list of short term positions
    - [x] PUT       `/portfolio/positions`          Convert the margin product of an open position
    - [x] GET       `/portfolio/holdings/auctions`  Retrieve the list of auctions that are currently being held
    - [ ] POST      `/portfolio/holdings/authorise` Place an electronic authorisation to debit shares and settle the transactions
- [x] **Market quotes and instruments**
    - [x] GET       `/instruments`                  Retrieve the CSV dump of all tradable instruments
    - [x] GET       `/instruments/:exchange`        Retrieve the CSV dump of instruments in the particular exchange
    - [x] GET       `/quote`                        Retrieve the full market quotes for one or more instruments
    - [x] GET       `/quote/ohlc`                   Retrieve OHLC quotes for one or more instruments
    - [x] GET       `/quote/ltp`                    Retrieve LTP quotes for one or more instruments
- [ ] **Historical candle data**
    - [ ] GET       `/instruments/historical/:instrument_token/:interval`   Retrieve historical candle records for a given instrument
- [ ] **Mutual funds**
    - [ ] POST      `/mf/orders`                    Place a buy or sell order
    - [ ] DELETE    `/mf/orders/:order_id`          Cancel an open or pending order
    - [ ] GET       `/mf/orders`                    Retrieve the list of all orders (open and executed) over the last 7 days
    - [ ] GET       `/mf/orders/:order_id`          Retrieve an individual order
    - [ ] POST      `/mf/sips`                      Place a SIP order
    - [ ] PUT       `/mf/sips/:order_id`            Modify an open SIP order
    - [ ] DELETE    `/mf/sips/:order_id`            Cancel an open SIP order
    - [ ] GET       `/mf/sips`                      Retrieve the list of all open SIP orders
    - [ ] GET       `/mf/sips/:order_id`            Retrieve an individual SIP order
    - [ ] GET       `/mf/holdings`                  Retrieve the list of mutual fund holdings available in the DEMAT
    - [ ] GET       `/mf/instruments`               Retrieve the master list of all mutual funds available on the platform
- [x] **Margin calculation**
    - [x] POST      `/margins/orders`               Calculates margins for each order considering the existing positions and open orders
    - [x] POST      `/margins/basket`               Calculates margins for spread orders
    - [x] POST      `/charges/orders`               Calculates order-wise charges for orderbook

- [x] **WebSocket streaming**
    - [x] Auto-reconnect mechanism with subscription 

### Disclaimer

**Important Notice**:

* The `manja` crate is currently in development and should be considered unstable. The API is subject to change without notice, and breaking changes are likely to occur.

* The software is provided "as-is" without any warranties, express or implied. The author and contributors of this SDK do not take responsibility for any financial losses, damages, or other issues that may arise from the use of this project.
