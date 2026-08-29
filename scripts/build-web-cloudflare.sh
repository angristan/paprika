#!/usr/bin/env bash
set -euo pipefail

# Workers Builds caches Bun's global cache. Keep verified tools, Cargo data, and
# content-addressed WASM artifacts below it so a JS-only change can reuse the
# exact package while changed Rust inputs always produce a cache miss.
readonly rust_version="1.97.1"
cache_root="${BUN_INSTALL_CACHE_DIR:-$HOME/.bun/install/cache}/paprika"
export PAPRIKA_TOOLS_DIR="$cache_root/tools"
export RUSTUP_HOME="$cache_root/rustup"
export CARGO_HOME="$cache_root/cargo"
export CARGO_TARGET_DIR="$cache_root/target"
export PAPRIKA_WASM_CACHE_DIR="$cache_root/wasm-packages"
export RUSTUP_TOOLCHAIN="$rust_version"
export PATH="$CARGO_HOME/bin:$PAPRIKA_TOOLS_DIR/bin:$PATH"

mkdir -p "$PAPRIKA_TOOLS_DIR/bin" "$CARGO_HOME" "$CARGO_TARGET_DIR" \
  "$PAPRIKA_WASM_CACHE_DIR"

# Download a versioned rustup-init binary and verify its pinned digest. Never
# execute a network response directly in a shell pipeline.
bash scripts/install-rust-toolchain.sh
bash scripts/install-wasm-pack.sh "$PAPRIKA_TOOLS_DIR/bin"
bash scripts/build-web.sh
