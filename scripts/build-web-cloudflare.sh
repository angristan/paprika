#!/usr/bin/env bash
set -euo pipefail

# Workers Builds does not currently include Rust. Install the pinned toolchain
# into its ephemeral home, then use an integrity-checked wasm-pack release.
rust_version="1.97.1"
wasm_pack_version="0.15.0"
wasm_pack_sha256="c09f971ecaed9a2efc80fdcea7a00ef6b53c7fadc8c57d1f61b53a6aa66b668a"
tools_dir="${XDG_CACHE_HOME:-$HOME/.cache}/paprika-tools"
export RUSTUP_HOME="${RUSTUP_HOME:-$tools_dir/rustup}"
export CARGO_HOME="${CARGO_HOME:-$tools_dir/cargo}"
export RUSTUP_TOOLCHAIN="$rust_version"
export PATH="$CARGO_HOME/bin:$tools_dir/bin:$PATH"

mkdir -p "$tools_dir/bin" "$CARGO_HOME"

if ! command -v rustup >/dev/null 2>&1; then
  curl --fail --proto '=https' --tlsv1.2 --silent --show-error \
    https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile minimal \
      --default-toolchain "$rust_version" \
      --target wasm32-unknown-unknown
else
  rustup toolchain install "$rust_version" --profile minimal --no-self-update
  rustup target add wasm32-unknown-unknown --toolchain "$rust_version"
fi

if [[ "$(wasm-pack --version 2>/dev/null || true)" != "wasm-pack $wasm_pack_version" ]]; then
  archive="$tools_dir/wasm-pack.tar.gz"
  url="https://github.com/wasm-bindgen/wasm-pack/releases/download/v${wasm_pack_version}/wasm-pack-v${wasm_pack_version}-x86_64-unknown-linux-musl.tar.gz"
  curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error \
    --retry 3 --output "$archive" "$url"
  printf '%s  %s\n' "$wasm_pack_sha256" "$archive" | sha256sum --check --status
  tar --extract --gzip --file "$archive" --directory "$tools_dir/bin" \
    --strip-components=1 \
    "wasm-pack-v${wasm_pack_version}-x86_64-unknown-linux-musl/wasm-pack"
fi

bash scripts/build-web.sh
