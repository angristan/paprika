#!/usr/bin/env bash
set -euo pipefail

# This bootstrap is for Linux x86-64 CI images. Developers with rustup already
# installed still use the repository's pinned rust-toolchain.toml.
readonly rust_version="1.97.1"
readonly rustup_version="1.28.2"
# Published beside the binary at static.rust-lang.org/rustup/archive/1.28.2/.
readonly rustup_init_sha256="20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c"
readonly rustup_target="x86_64-unknown-linux-gnu"
readonly wasm_target="wasm32-unknown-unknown"

export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export PATH="$CARGO_HOME/bin:$PATH"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if ! grep -Fqx "channel = \"$rust_version\"" "$root/rust-toolchain.toml"; then
  echo "rust-toolchain.toml does not match the bootstrap's Rust $rust_version pin" >&2
  exit 1
fi

if ! command -v rustup >/dev/null 2>&1; then
  if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
    echo "automatic rustup installation only supports Linux x86-64" >&2
    echo "install rustup for this platform, then rerun this script" >&2
    exit 1
  fi

  tools_dir="${PAPRIKA_TOOLS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/paprika-tools}"
  download_dir="$tools_dir/downloads"
  installer="$download_dir/rustup-init-$rustup_version-$rustup_target"
  temporary="$installer.download.$$"
  mkdir -p "$download_dir" "$CARGO_HOME"

  cleanup() {
    rm -f "$temporary"
  }
  trap cleanup EXIT

  curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error \
    --retry 3 --output "$temporary" \
    "https://static.rust-lang.org/rustup/archive/$rustup_version/$rustup_target/rustup-init"
  printf '%s  %s\n' "$rustup_init_sha256" "$temporary" | sha256sum --check --status
  install -m 0755 "$temporary" "$installer"

  "$installer" -y --no-modify-path --profile minimal \
    --default-toolchain "$rust_version"
fi

rustup toolchain install "$rust_version" \
  --profile minimal \
  --no-self-update \
  --component clippy \
  --component rustfmt \
  --target "$wasm_target"

rustup run "$rust_version" rustc --version
