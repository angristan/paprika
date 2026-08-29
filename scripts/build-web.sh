#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

command -v wasm-pack >/dev/null 2>&1 || {
  echo "wasm-pack 0.15.0 is required (on Arch Linux: pacman -S wasm-pack)" >&2
  exit 1
}
[[ "$(wasm-pack --version)" == "wasm-pack 0.15.0" ]] || {
  echo "wasm-pack 0.15.0 is required; found $(wasm-pack --version)" >&2
  exit 1
}

mkdir -p web/dist
cp web/src/index.html web/src/styles.css web/src/app.js web/src/converter.worker.js \
  web/src/epub-preview.js web/src/favicon.svg web/src/_headers web/src/404.html web/dist/
cp LICENSE-APACHE LICENSE-MIT web/dist/

# Rust already optimizes this release for size. wasm-opt saves about 15% more
# bytes but adds roughly six minutes on this module, so CI and Cloudflare skip
# it by default. Set PAPRIKA_WASM_OPT=1 for an explicitly size-minimized build.
wasm_opt_args=(--no-opt)
if [[ "${PAPRIKA_WASM_OPT:-0}" == "1" ]]; then
  wasm_opt_args=()
fi

wasm_cache_dir=""
wasm_cache_key=""
if [[ -n "${PAPRIKA_WASM_CACHE_DIR:-}" ]]; then
  command -v sha256sum >/dev/null 2>&1 || {
    echo "sha256sum is required when PAPRIKA_WASM_CACHE_DIR is set" >&2
    exit 1
  }
  wasm_cache_key="$({
    printf 'wasm-pack=%s\n' "$(wasm-pack --version)"
    printf 'rustc=%s\n' "$(rustc -vV)"
    printf 'wasm-opt=%s\n' "${PAPRIKA_WASM_OPT:-0}"
    printf 'rustflags=%s\n' "${RUSTFLAGS:-}"
    while IFS= read -r input; do
      sha256sum "$input"
    done < <(
      {
        printf '%s\n' Cargo.lock Cargo.toml rust-toolchain.toml scripts/build-web.sh
        find crates -type f \( -name '*.rs' -o -name Cargo.toml \) -print
      } | LC_ALL=C sort
    )
  } | sha256sum | awk '{print $1}')"
  wasm_cache_dir="$PAPRIKA_WASM_CACHE_DIR/$wasm_cache_key"
fi

cache_is_valid() {
  [[ -n "$wasm_cache_dir" && -f "$wasm_cache_dir/SHA256SUMS" ]] || return 1
  for asset in \
    .gitignore package.json paprika_wasm.d.ts paprika_wasm.js \
    paprika_wasm_bg.wasm paprika_wasm_bg.wasm.d.ts; do
    [[ -f "$wasm_cache_dir/pkg/$asset" ]] || return 1
  done
  (cd "$wasm_cache_dir/pkg" && sha256sum --check --status ../SHA256SUMS)
}

if cache_is_valid; then
  echo "Restoring verified WebAssembly package $wasm_cache_key"
  mkdir -p web/dist/pkg
  cp "$wasm_cache_dir"/pkg/* web/dist/pkg/
  cp "$wasm_cache_dir"/pkg/.gitignore web/dist/pkg/
else
  RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-feature=+simd128" \
    wasm-pack build \
    --target web \
    --release \
    "${wasm_opt_args[@]}" \
    --out-dir "$root/web/dist/pkg" \
    --out-name paprika_wasm \
    crates/paprika-wasm \
    --locked

  if [[ -n "$wasm_cache_dir" ]]; then
    mkdir -p "$wasm_cache_dir/pkg"
    cp web/dist/pkg/* "$wasm_cache_dir/pkg/"
    cp web/dist/pkg/.gitignore "$wasm_cache_dir/pkg/"
    (
      cd "$wasm_cache_dir/pkg"
      find . -type f -print | LC_ALL=C sort | while IFS= read -r asset; do
        sha256sum "$asset"
      done > ../SHA256SUMS
    )
    cache_is_valid || {
      echo "new WebAssembly cache entry failed verification" >&2
      exit 1
    }
  fi
fi

bash scripts/check-dist.sh
