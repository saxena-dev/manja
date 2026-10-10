# Contributing to manja

Thanks for helping. Bug reports, questions, documentation fixes and code are all
welcome, and there is no formal process: open an issue or a pull request, and we'll take
it from there. For a large change, a quick issue first saves you from building something
that doesn't fit.

> **Don't paste credentials or account data.** API keys, access tokens, API secrets and
> request tokens don't belong in issues, pull requests or test fixtures, even expired
> ones. manja's credential types redact themselves, but errors can include Kite's own
> messages, which may mention amounts or order IDs, so review anything before you share
> it.

## Setting up

The tests use Zerodha's official mock responses, kept in a Git submodule:

```text
git clone --recurse-submodules https://github.com/saxena-dev/manja.git
```

For an existing clone, run `git submodule update --init`. The verification scripts use
two toolchains, the minimum supported version and stable:

```text
rustup toolchain install 1.88.0 stable
```

The scripts are Bash and use common Unix tools, so on Windows, run them under WSL.

## Checking your change

While you work, run what you touched:

```text
cargo test --test http_orders                              # one test file
cargo test --no-default-features --features decoder        # one feature configuration
```

Before opening a pull request, run the full offline gate: formatting, the docs,
Clippy on every feature configuration, the minimum-version builds, the tests and the
packaging check.

```text
scripts/quick-verification.sh
```

Every test runs against local servers on `127.0.0.1`, so none needs a Kite account or
network access. Please keep new tests that way.

`scripts/full-verification.sh` runs the tests on every feature configuration and
toolchain, and also re-checks the recorded Kite documentation pages, which needs network
access. `scripts/quick-verification.sh --coverage` measures coverage and needs
`cargo-llvm-cov` (`cargo install cargo-llvm-cov`, with
`rustup component add llvm-tools-preview`). You don't need either for a pull request.

## A few things worth knowing

- **Tests:** new behavior needs a test, and a bug fix is best shown by a test that
  fails without it.
- **Fixtures:** never edit the files in `kiteconnect-mocks/`; they are Zerodha's, served
  byte for byte. For an HTTP response or text message the mocks don't have, write it
  inline in the test and label it "Supplemental", saying which official file it was
  derived from. Binary ticker messages go in `tests/fixtures/ticker/`, with an entry in
  its `MANIFEST.toml`. `docs/verification.md` §1.2 and `docs/verification.md` §1.3 have
  the details.
- **Docs:** a change in behavior updates `docs/contract.md`, which specifies what users
  can rely on, and a notable change gets a line under "Unreleased" in `CHANGELOG.md`.
- **Citing Kite:** code that follows Kite's documentation cites the page and lines, as
  `kite:orders.md:58-103`, for a page listed in `docs/kite-sources.toml`.
  `tests/references.rs` checks these.

manja's design comes down to a few rules: a failure is never reported as success, a
call that can change an account is never retried, everything is bounded, unknown values
from Kite are kept rather than guessed, and no secret appears in output. If a change
bends one, say so in the pull request.

## Security issues

Please report a vulnerability privately rather than in a public issue; see
[`SECURITY.md`](SECURITY.md).

## License

Contributions are licensed under the [MIT License](LICENSE), like the rest of manja.
