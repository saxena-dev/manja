#!/usr/bin/env bash
# Full verification: the quick gates, every feature row on the minimum and
# the newer supported toolchain, the doc tests, the packaged crate, and the
# recorded Kite documentation pages.
#
#   scripts/full-verification.sh
#
# Output goes to the terminal and to a timestamped file under
# target/verification/, which Git ignores and Cargo never packages.
# Exits non-zero if any step fails.
# The Kite source check needs network access to kite.trade.
set -u

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

msrv=1.95.0
latest=1.98.0
out_dir="$root/target/verification"
mkdir -p "$out_dir"
log="$out_dir/full-verification-$(date -u +%Y%m%dT%H%M%SZ).txt"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

# Everything below also goes to the log.
exec > >(tee "$log") 2>&1

failed=()
run() {
  echo "== $*"
  if "$@"; then
    echo "-- ok: $*"
  else
    echo "-- FAIL: $*"
    failed+=("$*")
  fi
}

echo "full verification of $(git rev-parse --short HEAD) at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "log: $log"

# Quick gates, pinned to the newer toolchain.
run cargo +"$latest" fmt --all -- --check
run cargo +"$latest" clippy --offline --all-targets --all-features -- -D warnings
run env RUSTDOCFLAGS="-D warnings" cargo +"$latest" doc --offline --no-deps

# The eight feature rows on both toolchains. Each row is an array, so no
# shell has to split a string into separate arguments.
rows=(none decoder http ticker ticker,decoder http,ticker http,ticker,decoder default)
for tc in "$msrv" "$latest"; do
  for row in "${rows[@]}"; do
    case "$row" in
      none) args=(--no-default-features) ;;
      default) args=() ;;
      *) args=(--no-default-features --features "$row") ;;
    esac
    # ${args[@]+...} keeps an empty row valid under `set -u` in bash 3.2.
    run cargo +"$tc" test --offline ${args[@]+"${args[@]}"}
  done
done

run cargo +"$latest" test --offline --doc
run tests/packaging/check.sh "$scratch/pkg"
run scripts/verify-kite-sources.sh "$scratch/kite"

echo
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
