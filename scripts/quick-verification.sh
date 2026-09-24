#!/usr/bin/env bash
# Quick verification: the same coverage as full-verification.sh in a fraction
# of the time.
#
#   scripts/quick-verification.sh            everything that runs offline
#   scripts/quick-verification.sh --online   also re-check the Kite pages
#   scripts/quick-verification.sh --coverage also measure test coverage
#
# The feature rows and the minimum toolchain differ only in what compiles, so
# they are compile-checked (clippy, all targets) instead of tested. The tests,
# doc tests included, run once with every feature enabled. full-verification.sh
# still runs the test suite on every row and toolchain; use it before a
# release.
#
# Output goes to the terminal and to a timestamped file under
# target/verification/. Exits non-zero if any step fails.
set -u

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

msrv=1.88.0
latest=stable
online=0
coverage=0
for arg in "$@"; do
  case "$arg" in
    --online) online=1 ;;
    --coverage) coverage=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

out_dir="$root/target/verification"
mkdir -p "$out_dir"
log="$out_dir/quick-verification-$(date -u +%Y%m%dT%H%M%SZ).txt"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
exec > >(tee "$log") 2>&1

failed=()
run() {
  echo "== $*"
  local start=$SECONDS
  if "$@"; then
    echo "-- ok ($((SECONDS - start))s): $*"
  else
    echo "-- FAIL ($((SECONDS - start))s): $*"
    failed+=("$*")
  fi
}

echo "quick verification of $(git rev-parse --short HEAD) at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "log: $log"
begin=$SECONDS

# Formatting and documentation.
run cargo +"$latest" fmt --all -- --check
run env RUSTDOCFLAGS="-D warnings" cargo +"$latest" doc --offline --no-deps

# Every feature row compiles cleanly, tests and examples included.
rows=(none decoder http ticker ticker,decoder http,ticker http,ticker,decoder default)
for row in "${rows[@]}"; do
  case "$row" in
    none) args=(--no-default-features) ;;
    default) args=() ;;
    *) args=(--no-default-features --features "$row") ;;
  esac
  # ${args[@]+...} keeps an empty row valid under `set -u` in bash 3.2.
  run cargo +"$latest" clippy --offline --all-targets ${args[@]+"${args[@]}"} -- -D warnings
done

# The minimum supported toolchain builds the smallest and the largest rows.
run cargo +"$msrv" check --offline --all-targets --no-default-features
run cargo +"$msrv" check --offline --all-targets --all-features

# Every test and doc test, once. They build in their own target directory:
# on macOS each test process that builds an HTTP client asks the system for
# its proxy settings, and that lookup scans the directory holding the
# executable. target/debug/deps collects every row and toolchain of the
# other steps (hundreds of thousands of files), which costs seconds per test
# binary; this directory only ever holds one configuration.
run env CARGO_TARGET_DIR="$root/target/quick-test" cargo +"$latest" test --offline --all-features

# Line and region coverage of the all-features test run, with cargo-llvm-cov
# (cargo install cargo-llvm-cov; rustup component add llvm-tools-preview).
# The instrumented build keeps to its own target directory, target/llvm-cov-target.
# The HTML report is written to target/verification/coverage/html/index.html.
if [ "$coverage" = 1 ]; then
  run cargo +"$latest" llvm-cov clean --workspace
  run cargo +"$latest" llvm-cov --offline --all-features --no-report
  run cargo +"$latest" llvm-cov report --summary-only
  run cargo +"$latest" llvm-cov report --html --output-dir "$out_dir/coverage"
else
  echo "== skipped: coverage (pass --coverage to run it)"
fi

# The packaged crate builds and runs for a downstream consumer.
run tests/packaging/check.sh "$scratch/pkg"

if [ "$online" = 1 ]; then
  run scripts/verify-kite-sources.sh "$scratch/kite"
else
  echo "== skipped: Kite source check (pass --online to run it)"
fi

echo
echo "elapsed: $((SECONDS - begin))s"
if [ ${#failed[@]} -eq 0 ]; then
  echo "== done: ALL PASSED"
  status=0
else
  echo "== done: ${#failed[@]} FAILED"
  printf '   %s\n' "${failed[@]}"
  status=1
fi
echo "log: $log"
exit $status
