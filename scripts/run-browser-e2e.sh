#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

[[ -x node_modules/.bin/playwright ]] || {
  echo "Playwright is not installed; run bun install --frozen-lockfile" >&2
  exit 1
}

if [[ "${PAPRIKA_E2E_SKIP_BUILD:-0}" != "1" ]]; then
  bun run build
fi

# The Playwright config always starts Wrangler on loopback and refuses to reuse
# an existing server. A branch test therefore cannot target production or a
# preview deployment by accident.
bun run playwright test
