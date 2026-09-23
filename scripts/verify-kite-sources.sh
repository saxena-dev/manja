#!/usr/bin/env bash
# Fetch every page listed in docs/kite-sources.toml and compare it with the
# recorded SHA-256, size and line count.
#
#   scripts/verify-kite-sources.sh [download-dir]
#
# Needs network access to kite.trade. Exits non-zero if any page cannot be
# fetched or no longer matches; the fetched copies are kept in download-dir
# (a temporary directory by default) so a changed page can be diffed and
# the citations to it re-checked.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
manifest="$root/docs/kite-sources.toml"
out=${1:-$(mktemp -d)}
mkdir -p "$out"

status=0
while IFS='|' read -r name url sha bytes lines; do
  file="$out/$name"
  if ! curl -fsSL -o "$file" "$url"; then
    echo "FAIL  $name: could not fetch $url"; status=1; continue
  fi
  got_sha=$(shasum -a 256 "$file" | cut -d' ' -f1)
  got_bytes=$(wc -c <"$file" | tr -d ' ')
  got_lines=$(wc -l <"$file" | tr -d ' ')
  if [[ $got_sha == "$sha" && $got_bytes == "$bytes" && $got_lines == "$lines" ]]; then
    echo "ok    $name"
  else
    echo "DRIFT $name: sha256 $got_sha, $got_bytes bytes, $got_lines lines (recorded $sha, $bytes, $lines)"
    status=1
  fi
done < <(awk -F' = ' '
  /^\[\[page\]\]/ { if (name) print name "|" url "|" sha "|" bytes "|" lines; name="" }
  $1 == "name"   { gsub(/"/, "", $2); name = $2 }
  $1 == "url"    { gsub(/"/, "", $2); url = $2 }
  $1 == "sha256" { gsub(/"/, "", $2); sha = $2 }
  $1 == "bytes"  { bytes = $2 }
  $1 == "lines"  { lines = $2 }
  END { if (name) print name "|" url "|" sha "|" bytes "|" lines }
' "$manifest")

echo "fetched copies: $out"
exit $status
