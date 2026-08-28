#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

command -v wasm-pack >/dev/null 2>&1 || {
  echo "wasm-pack is required (on Arch Linux: pacman -S wasm-pack)" >&2
  exit 1
}

mkdir -p web/dist
cp web/src/index.html web/src/styles.css web/src/app.js web/src/converter.worker.js \
  web/src/favicon.svg web/src/_headers web/src/404.html web/dist/

RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-feature=+simd128" \
  wasm-pack build crates/paprika-wasm \
  --target web \
  --release \
  --out-dir "$root/web/dist/pkg" \
  --out-name paprika_wasm

# The assets directory is intentionally not deleted: a developer may have put
# local evidence there. Refuse to deploy unknown or obsolete files instead.
while IFS= read -r asset; do
  case "$asset" in
    404.html|_headers|app.js|converter.worker.js|favicon.svg|index.html|styles.css) ;;
    pkg/.gitignore|pkg/package.json|pkg/paprika_wasm.d.ts|pkg/paprika_wasm.js) ;;
    pkg/paprika_wasm_bg.wasm|pkg/paprika_wasm_bg.wasm.d.ts) ;;
    *)
      echo "Unexpected file in web/dist: $asset" >&2
      echo "Review and move or remove it before building." >&2
      exit 1
      ;;
  esac
done < <(find web/dist -type f -printf '%P\n' | sort)
