#!/usr/bin/env bash
# Packaged-crate validation (plan task S30). Offline; no registry command.
#
#   tests/packaging/check.sh [scratch-dir]
#
# 1. Lists the package and fails if a fixture, mock, test, example or bench
#    would ship.
# 2. Packages without verification and unpacks the .crate into scratch.
# 3. Builds and runs an out-of-tree consumer against the unpacked package
#    for the default, http, ticker and decoder rows, and checks each row's
#    resolved normal graph.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
scratch=${1:-$(mktemp -d)}
mkdir -p "$scratch"
cd "$root"

list=$(cargo package --offline --allow-dirty --list)
for forbidden in kiteconnect-mocks tests/ examples/ benches/ .tracker .beads; do
  if grep -q "^$forbidden" <<<"$list"; then
    echo "FAIL: $forbidden would be packaged"; exit 1
  fi
done
echo "package list: $(wc -l <<<"$list" | tr -d ' ') files, no fixture, mock, test, example or bench"

cargo package --offline --allow-dirty --no-verify >/dev/null 2>&1
crate=$(ls target/package/manja-*.crate | tail -1)
shasum -a 256 "$crate"
rm -rf "$scratch/unpacked" && mkdir -p "$scratch/unpacked"
tar -xzf "$crate" -C "$scratch/unpacked"
pkg=$(ls -d "$scratch"/unpacked/manja-* | tail -1)
test ! -e "$pkg/tests" && test ! -e "$pkg/kiteconnect-mocks"

rm -rf "$scratch/consumer" && cp -R tests/packaging/consumer "$scratch/consumer"
sed "s|@MANJA@|$pkg|" "$scratch/consumer/Cargo.toml.in" > "$scratch/consumer/Cargo.toml"
rm "$scratch/consumer/Cargo.toml.in"
export CARGO_TARGET_DIR="$scratch/target"
manifest="$scratch/consumer/Cargo.toml"

run_row() {
  local name=$1; shift
  cargo run --offline -q --manifest-path "$manifest" "$@"
  local tree
  tree=$(cargo tree --offline --manifest-path "$manifest" -e normal --prefix none "$@" | sort -u)
  for banned in fantoccini totp-rs base32 tracing-subscriber dotenv; do
    if grep -q "^$banned v" <<<"$tree"; then echo "FAIL [$name]: $banned"; exit 1; fi
  done
  case $name in
    decoder)
      for banned in tokio reqwest hyper tungstenite; do
        if grep -q "^$banned v" <<<"$tree"; then echo "FAIL [decoder]: $banned"; exit 1; fi
      done ;;
    http)
      if grep -q "^tungstenite v" <<<"$tree"; then echo "FAIL [http]: tungstenite"; exit 1; fi ;;
    ticker)
      if grep -q "^reqwest v" <<<"$tree"; then echo "FAIL [ticker]: reqwest"; exit 1; fi ;;
  esac
  echo "row $name: ok"
}

# The documented examples against the packaged crate. They read fixtures
# relative to the consuming crate, so the test-only assets are copied in
# beside them; none of them is part of the package.
cp -R examples "$scratch/consumer/examples"
mkdir -p "$scratch/consumer/tests/fixtures"
cp -R kiteconnect-mocks "$scratch/consumer/kiteconnect-mocks"
cp -R tests/fixtures/ticker "$scratch/consumer/tests/fixtures/ticker"
cargo build --offline -q --manifest-path "$manifest" --examples
for example in http_client session ticker ticker_typed decode_offline; do
  cargo run --offline -q --manifest-path "$manifest" --example "$example" >/dev/null
  echo "example $example: ok"
done

run_row default
run_row http --no-default-features --features http
run_row ticker --no-default-features --features ticker
run_row decoder --no-default-features --features decoder
echo "packaged consumer: ok"
