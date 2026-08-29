#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
command -v bun >/dev/null 2>&1 || {
  echo "Bun is required" >&2
  exit 1
}

output="$(mktemp -d "${TMPDIR:-/tmp}/paprika-js-check.XXXXXX")"
trap 'rm -rf "$output"' EXIT

# Bundling parses every production module without executing browser globals.
# The generated wasm-pack module is external because it does not exist until
# the production build step.
bun build \
  web/src/app.js \
  web/src/converter.worker.js \
  --target=browser \
  --outdir="$output" \
  --external='*paprika_wasm.js'

# Static entry points must remain local and explicit.
for asset in styles.css app.js favicon.svg; do
  grep -Fq "/$asset" web/src/index.html || {
    echo "web/src/index.html does not reference /$asset" >&2
    exit 1
  }
done

grep -Fq 'connect-src '\''self'\''' web/src/_headers || {
  echo "the CSP must keep browser network connections same-origin" >&2
  exit 1
}
