# Changelog

Notable changes to manja, newest first. Before 1.0, breaking changes ship only in
minor-version bumps.

## Unreleased

## 0.2.0 - 2026-09-25

The first release of manja as it is today: a bounded, observable Rust SDK for Zerodha's
Kite Connect API, built so that nothing fails quietly. The behavior you can rely on is
specified in [`docs/contract.md`](docs/contract.md).

- **HTTP API:** accounts, orders, GTT orders, portfolio and holdings authorisation,
  quotes and instruments, historical candles, mutual fund reads, and margin and charge
  calculations, with validated request types and checked ID types.
- **Typed errors.** Every failed HTTP call returns an `HttpError` with its kind, how far
  the request got, the HTTP status, and Kite's error type and message.
- **Safe retries.** Calls that can change your account make exactly one attempt. Reads
  and calculations retry transient failures, within a deadline.
- **Local rate limiting** for every documented Kite Connect limit, with dispatch permits
  that separate waiting for capacity from sending.
- **Sessions:** request-token exchange and session invalidation, with credentials kept
  out of `Debug` output, errors, spans and metrics.
- **A supervised ticker** that restores subscriptions on every reconnect, delivers every
  message it accepts in source order, and ends with an explicit error rather than
  dropping data.
- **A decoder** for ticker packets, with market depth, and for order updates, live or
  on captured data, with no runtime needed.
- **Opt-in observability** with your own metrics recorder and `tracing` subscriber;
  nothing is installed globally.
- **Feature slices:** `http`, `ticker` and `decoder` are independent, and a build with
  only `decoder` has no Tokio or network dependency.
- **Documentation:** a README whose examples compile as doc tests, and an example on
  every HTTP resource and on the main ticker, decoder and observability types.
- Rust edition 2024, with a minimum supported Rust version of 1.88.0.
