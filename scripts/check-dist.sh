#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

[[ -d web/dist ]] || {
  echo "web/dist does not exist; run bun run build first" >&2
  exit 1
}

while IFS= read -r path; do
  asset="${path#./}"
  case "$asset" in
    404.html|_headers|app.js|converter.worker.js|epub-preview.js|favicon.svg|index.html|styles.css) ;;
    LICENSE-APACHE|LICENSE-MIT) ;;
    pkg/.gitignore|pkg/package.json|pkg/paprika_wasm.d.ts|pkg/paprika_wasm.js) ;;
    pkg/paprika_wasm_bg.wasm|pkg/paprika_wasm_bg.wasm.d.ts) ;;
    *)
      echo "Unexpected file in web/dist: $asset" >&2
      echo "Review and move or remove it before building." >&2
      exit 1
      ;;
  esac
done < <(cd web/dist && find . -type f -print | LC_ALL=C sort)

for asset in \
  404.html _headers app.js converter.worker.js epub-preview.js favicon.svg \
  index.html styles.css LICENSE-APACHE LICENSE-MIT \
  pkg/package.json pkg/paprika_wasm.js pkg/paprika_wasm_bg.wasm; do
  [[ -f "web/dist/$asset" ]] || {
    echo "Missing required deploy asset: $asset" >&2
    exit 1
  }
done

cmp -s LICENSE-APACHE web/dist/LICENSE-APACHE || {
  echo "web/dist/LICENSE-APACHE does not match the source license" >&2
  exit 1
}
cmp -s LICENSE-MIT web/dist/LICENSE-MIT || {
  echo "web/dist/LICENSE-MIT does not match the source license" >&2
  exit 1
}
