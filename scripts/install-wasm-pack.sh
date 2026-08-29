#!/usr/bin/env bash
set -euo pipefail

readonly wasm_pack_version="0.15.0"
readonly wasm_pack_target="x86_64-unknown-linux-musl"
# Matches the digest published for the v0.15.0 GitHub release asset.
readonly wasm_pack_sha256="c09f971ecaed9a2efc80fdcea7a00ef6b53c7fadc8c57d1f61b53a6aa66b668a"
readonly wasm_pack_binary_sha256="c6c3d54702f4bae4a1d51e37e19c2c61b130865dc3fabc745eebe8194b87b253"

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "automatic wasm-pack installation only supports Linux x86-64" >&2
  echo "install wasm-pack $wasm_pack_version for this platform" >&2
  exit 1
fi

bin_dir="${1:-${PAPRIKA_TOOLS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/paprika-tools}/bin}"
tool="$bin_dir/wasm-pack"
if [[ -x "$tool" ]] \
  && printf '%s  %s\n' "$wasm_pack_binary_sha256" "$tool" | sha256sum --check --status \
  && [[ "$($tool --version)" == "wasm-pack $wasm_pack_version" ]]; then
  exit 0
fi

archive_name="wasm-pack-v$wasm_pack_version-$wasm_pack_target.tar.gz"
archive_dir="${PAPRIKA_TOOLS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/paprika-tools}/downloads"
archive="$archive_dir/$archive_name"
temporary="$archive.download.$$"
staging=""
mkdir -p "$archive_dir" "$bin_dir"

cleanup() {
  rm -f "$temporary"
  if [[ -n "$staging" && -d "$staging" ]]; then
    rm -rf "$staging"
  fi
}
trap cleanup EXIT

curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error \
  --retry 3 --output "$temporary" \
  "https://github.com/wasm-bindgen/wasm-pack/releases/download/v$wasm_pack_version/$archive_name"
printf '%s  %s\n' "$wasm_pack_sha256" "$temporary" | sha256sum --check --status
mv "$temporary" "$archive"

staging="$(mktemp -d "${TMPDIR:-/tmp}/paprika-wasm-pack.XXXXXX")"
tar --extract --gzip --file "$archive" --directory "$staging" \
  --strip-components=1 \
  "wasm-pack-v$wasm_pack_version-$wasm_pack_target/wasm-pack"
printf '%s  %s\n' "$wasm_pack_binary_sha256" "$staging/wasm-pack" \
  | sha256sum --check --status
install -m 0755 "$staging/wasm-pack" "$tool"
"$tool" --version
