#!/usr/bin/env bash
set -euo pipefail

readonly epubcheck_version="5.3.0"
# Matches the digest published for the v5.3.0 GitHub release asset.
readonly epubcheck_sha256="6c07e68584b2e2ce2f89fe06e1246dfead3eb36b46b340e7d93524f29dcff6c5"
readonly epubcheck_jar_sha256="f7f96617c929371821609b88c8484d6dc9f24fe916499863c46094c5fb778a65"
readonly fixture_url="https://www.foundationdb.org/files/QuiCK.pdf"
readonly fixture_sha256="90b16b703c680aa90291d6008cdaadeaa7d604a3889ee5d3bb347db4c81a06db"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
command -v java >/dev/null 2>&1 || {
  echo "Java 17 or newer is required to run EPUBCheck $epubcheck_version" >&2
  exit 1
}

cache_root="${PAPRIKA_TOOLS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/paprika-tools}"
download_dir="$cache_root/downloads"
epubcheck_dir="$cache_root/epubcheck-$epubcheck_version"
epubcheck_jar="$epubcheck_dir/epubcheck.jar"
mkdir -p "$download_dir"

if [[ ! -f "$epubcheck_jar" ]] \
  || ! printf '%s  %s\n' "$epubcheck_jar_sha256" "$epubcheck_jar" \
    | sha256sum --check --status; then
  archive="$download_dir/epubcheck-$epubcheck_version.zip"
  temporary="$archive.download.$$"
  staging="$(mktemp -d "${TMPDIR:-/tmp}/paprika-epubcheck.XXXXXX")"
  cleanup_install() {
    rm -f "$temporary"
    rm -rf "$staging"
  }
  trap cleanup_install EXIT

  curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error \
    --retry 3 --output "$temporary" \
    "https://github.com/w3c/epubcheck/releases/download/v$epubcheck_version/epubcheck-$epubcheck_version.zip"
  printf '%s  %s\n' "$epubcheck_sha256" "$temporary" | sha256sum --check --status
  mv "$temporary" "$archive"
  if command -v unzip >/dev/null 2>&1; then
    unzip -q "$archive" -d "$staging"
  elif command -v bsdtar >/dev/null 2>&1; then
    bsdtar -xf "$archive" -C "$staging"
  else
    echo "unzip or bsdtar is required to install EPUBCheck" >&2
    exit 1
  fi
  [[ -f "$staging/epubcheck-$epubcheck_version/epubcheck.jar" ]] || {
    echo "EPUBCheck archive has an unexpected layout" >&2
    exit 1
  }
  printf '%s  %s\n' "$epubcheck_jar_sha256" \
    "$staging/epubcheck-$epubcheck_version/epubcheck.jar" \
    | sha256sum --check --status
  mkdir -p "$epubcheck_dir"
  cp -R "$staging/epubcheck-$epubcheck_version/." "$epubcheck_dir/"
  cleanup_install
  trap - EXIT
fi

if (( $# == 0 )); then
  fixture="$download_dir/QuiCK-$fixture_sha256.pdf"
  if [[ ! -f "$fixture" ]]; then
    fixture_download="$fixture.download.$$"
    trap 'rm -f "$fixture_download"' EXIT
    curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error \
      --retry 3 --output "$fixture_download" "$fixture_url"
    printf '%s  %s\n' "$fixture_sha256" "$fixture_download" | sha256sum --check --status
    mv "$fixture_download" "$fixture"
    trap - EXIT
  else
    printf '%s  %s\n' "$fixture_sha256" "$fixture" | sha256sum --check --status
  fi

  output_dir="$(mktemp -d "${TMPDIR:-/tmp}/paprika-epub-output.XXXXXX")"
  trap 'rm -rf "$output_dir"' EXIT
  output="$output_dir/quick.paprika.epub"
  cargo run --locked --release --bin paprika -- \
    "$fixture" --output "$output" --title "QuiCK regression fixture"
  set -- "$output"
fi

for epub in "$@"; do
  [[ -f "$epub" ]] || {
    echo "EPUB does not exist: $epub" >&2
    exit 1
  }
  java -jar "$epubcheck_jar" "$epub"
done
