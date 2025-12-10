# Manja Workspace Architecture

This repository is a Cargo workspace that hosts the `manja` facade crate and
several internal crates. Together they form a layered SDK for the
Zerodha Kite Connect APIs.

## Rust Toolchain & Edition

All crates in the workspace target the Rust 2024 edition:

- Each crate’s `Cargo.toml` sets `edition = "2024"`.
- The workspace is expected to be built with a stable Rust toolchain that supports the 2024 edition.

The top‑level workspace manifest (`Cargo.toml`) lists:

- `manja` – facade crate and example binary
- `manja-core` – transport‑agnostic models, errors, traits
- `manja-http` – HTTP transport and API groups
- `manja-ticker` – WebSocket ticker client
- `manja-extras` – optional WebDriver/TOTP login helpers

Most users should depend only on the `manja` crate. The other crates are
provided for advanced or highly specialized use‑cases.

## Crates and Responsibilities

- `manja`

  - Public facade crate published to crates.io.
  - Re‑exports core types (models, errors) from `manja-core`.
  - Wraps the lower‑level HTTP client from `manja-http` behind
    [`ManjaClient`](manja/src/client.rs).
  - Re‑exports the WebSocket ticker from `manja-ticker` when the
    `websocket` feature is enabled.
  - Re‑exports WebDriver/TOTP login helpers from `manja-extras` when the
    `webdriver-login` feature is enabled.
  - Builds the `manja` binary (`manja/src/main.rs`), which demonstrates a
    full login + REST + ticker flow using all features together.

- `manja-core`

  - Hosts shared, transport‑agnostic domain models and enums under
    `manja_core::models::*`.
  - Hosts core error types under `manja_core::error::*`.
  - Hosts trait definitions under `manja_core::traits::*` (e.g. helpers for
    building auth headers).
  - Has no dependency on async runtimes, HTTP clients, WebSockets, or
    WebDriver; suitable for reuse in non‑async or non‑networked code.

- `manja-http`

  - Hosts the HTTP transport layer for the SDK.
  - Provides the `HTTPClient` type and per‑domain API groups
    (`Session`, `User`, `Orders`, `Portfolio`, `Market`, `Margins`,
    `Charges`) under `manja_http::api`.
  - Depends on `reqwest` and related HTTP/backoff dependencies.
  - Used internally by the `manja` facade and can be depended on directly
    by advanced consumers who want more control than `ManjaClient` offers.

- `manja-ticker`

  - Hosts the async WebSocket ticker runtime and streaming types:
    `WebSocketClient`, `TickerStream`, `StreamState`, `KiteStreamCredentials`,
    ticker `Mode` and `TickerRequest`.
  - Depends on `tokio-tungstenite`, `tungstenite`, `stubborn-io`, etc.
  - Intended primarily as an implementation crate; most users should go
    through `manja::kite::ticker`, which re‑exports these types from the
    facade crate when the `websocket` feature is enabled.

- `manja-extras`
  - Hosts optional WebDriver + TOTP login helpers used to automate
    interactive Kite login flows.
  - Provides:
    - `browser_login_flow` – high‑level login flow driven by WebDriver.
    - `launch_browser` – Chrome/WebDriver launcher.
    - `generate_totp` – Base32 + TOTP generation.
  - Depends on heavier runtime dependencies like `fantoccini` and `totp-rs`.
  - Used by the `manja` facade when the `webdriver-login` feature is
    enabled, and also usable directly by advanced consumers.

## Facade vs Direct Dependencies

### Typical usage (facade crate only)

For most applications, depending only on the `manja` crate is sufficient:

```toml
[dependencies]
manja = "0.3"
```

This gives you:

- `ManjaClient` as a high‑level async client.
- Re‑exported models and error types (e.g. `UserProfile`, `KiteApiResponse`).
- Optional WebSocket ticker and WebDriver login flows via feature flags (see below).

### Minimal HTTP‑only usage

If you want to avoid WebSocket and WebDriver dependencies while keeping the
facade ergonomics, you can disable default features:

```toml
[dependencies]
manja = { version = "0.3", default-features = false }
```

This configuration keeps:

- The `manja` facade and `ManjaClient`.

and omits:

- WebSocket ticker integration (`websocket` feature).
- WebDriver/TOTP login helpers (`webdriver-login` feature).
- HTTP backoff/retry using the `backoff` crate (`backoff` feature).
- `dotenv`‑based configuration loading (`dotenv-config` feature).

You can selectively re‑enable any of these features as needed:

```toml
[dependencies]
manja = { version = "0.3", default-features = false, features = ["backoff"] }
```

### Direct access to core models

If you only need the data models and error types (for example, to share
types between services) without pulling in any HTTP/WebSocket clients, you
can depend on `manja-core` directly:

```toml
[dependencies]
manja-core = "0.3"
```

### Direct HTTP client usage

Advanced consumers who want to work directly with the underlying HTTP
client and API groups (for example, to implement custom facades or to plug
into an existing architecture) can depend on `manja-http`:

```toml
[dependencies]
manja-core = "0.3"
manja-http = "0.3"
```

This provides access to `manja_http::HTTPClient` and the domain‑specific
API modules (`Session`, `User`, `Orders`, etc.).

### Direct ticker client usage

If you want to integrate the WebSocket ticker into a custom async
pipeline, you can depend on `manja-ticker` directly:

```toml
[dependencies]
manja-core = "0.3"
manja-ticker = "0.3"
```

Most users should instead enable the `websocket` feature on the `manja`
facade and use `manja::kite::ticker`, which re‑exports these types.

### Direct WebDriver/TOTP login usage

If you need to orchestrate browser automation and login flows yourself
(outside of `ManjaClient`), you can depend on `manja-extras`:

```toml
[dependencies]
manja-core = "0.3"
manja-extras = "0.3"
```

This is equivalent to the advanced WebDriver login usage documented in the
README, but framed in terms of the workspace crates.

## Feature Flags and Layering

The main feature flags are defined on the `manja` facade crate
(`manja/Cargo.toml`):

- `websocket`

  - Enables WebSocket‑based ticker support.
  - Pulls in the `manja-ticker` crate and exposes its types under
    `manja::kite::ticker`.

- `webdriver-login`

  - Enables WebDriver/TOTP‑based login helpers.
  - Pulls in the `manja-extras` crate and exposes helpers under
    `manja::kite::login`.

- `backoff`

  - Enables `backoff`‑based retry logic for HTTP APIs.
  - Used in conjunction with `manja-http`’s own `backoff` feature.

- `dotenv-config`
  - Enables `dotenv`‑based configuration loading for development, tests,
    and examples.

The default feature set for `manja` is:

```toml
default = ["websocket", "backoff", "dotenv-config", "webdriver-login"]
```

which corresponds to:

- Full HTTP API coverage.
- WebSocket ticker support.
- WebDriver/TOTP login helpers.
- HTTP backoff/retry and `dotenv` integration.

For more details on usage patterns and code examples, see `README.md` and
the examples under `manja/examples`.
