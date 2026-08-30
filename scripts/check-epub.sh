#!/usr/bin/env bash
set -euo pipefail

readonly epubcheck_version="5.3.0"
# Matches the digest published for the v5.3.0 GitHub release asset.
readonly epubcheck_sha256="6c07e68584b2e2ce2f89fe06e1246dfead3eb36b46b340e7d93524f29dcff6c5"
readonly epubcheck_jar_sha256="f7f96617c929371821609b88c8484d6dc9f24fe916499863c46094c5fb778a65"
readonly -a fixture_names=("quick" "attention" "bert" "bitcoin")
readonly -a fixture_urls=(
  "https://www.foundationdb.org/files/QuiCK.pdf"
  "https://arxiv.org/pdf/1706.03762v7"
  "https://arxiv.org/pdf/1810.04805v2"
  "https://bitcoin.org/bitcoin.pdf"
)
readonly -a fixture_sha256s=(
  "90b16b703c680aa90291d6008cdaadeaa7d604a3889ee5d3bb347db4c81a06db"
  "bdfaa68d8984f0dc02beaca527b76f207d99b666d31d1da728ee0728182df697"
  "5692a5514787a8c6727b4ff3b726a3385798bc68e12138d1d4af83947e2acf6e"
  "b1674191a88ec5cdd733e4240a81803105dc412d6c6708d53ab94fc248f4f553"
)
readonly -a fixture_titles=(
  "QuiCK: A Queuing System in CloudKit"
  "Attention Is All You Need"
  "BERT: Pre-training of Deep Bidirectional Transformers for Language Understanding"
  "Bitcoin: A Peer-to-Peer Electronic Cash System"
)
readonly -a fixture_pages=(13 15 16 9)
readonly -a fixture_markers=(
  "FoundationDB Record Layer"
  "Multi-Head Attention"
  "masked language model"
  "peer-to-peer network"
)
readonly -a fixture_first_page_markers=(
  "Lev-Ari"
  "Vaswani"
  "Devlin"
  "Satoshi"
)

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

archive_member() {
  local archive="$1"
  local member="$2"
  if command -v unzip >/dev/null 2>&1; then
    unzip -p "$archive" "$member"
  else
    bsdtar -xOf "$archive" "$member"
  fi
}

archive_listing() {
  local archive="$1"
  if command -v unzip >/dev/null 2>&1; then
    unzip -Z1 "$archive"
  else
    bsdtar -tf "$archive"
  fi
}

download_fixture() {
  local name="$1"
  local url="$2"
  local sha256="$3"
  local fixture="$download_dir/$name-$sha256.pdf"
  if [[ ! -f "$fixture" ]]; then
    local temporary="$fixture.download.$$"
    if ! curl --fail --location --proto '=https' --tlsv1.2 --silent --show-error \
      --retry 3 --output "$temporary" "$url"; then
      rm -f "$temporary"
      return 1
    fi
    if ! printf '%s  %s\n' "$sha256" "$temporary" | sha256sum --check --status; then
      rm -f "$temporary"
      echo "$name fixture failed SHA-256 verification" >&2
      return 1
    fi
    mv "$temporary" "$fixture"
  else
    printf '%s  %s\n' "$sha256" "$fixture" | sha256sum --check --status
  fi
  printf '%s\n' "$fixture"
}

assert_headings() {
  local epub="$1"
  local member="$2"
  shift 2
  local body
  body="$(archive_member "$epub" "$member")"
  local heading
  for heading in "$@"; do
    grep -Fq "<h2>$heading</h2>" <<<"$body" || {
      echo "$member is missing heading: $heading" >&2
      return 1
    }
  done
}

assert_paper() {
  local name="$1"
  local epub="$2"
  local expected_title="$3"
  local expected_pages="$4"
  local expected_marker="$5"
  local first_page_marker="$6"
  local chapter
  local navigation
  chapter="$(archive_member "$epub" OEBPS/text/page-0001.xhtml)"
  navigation="$(archive_member "$epub" OEBPS/toc.xhtml)"
  grep -Fq "<title>$expected_title</title>" <<<"$chapter" || {
    echo "$name first chapter has the wrong document title" >&2
    return 1
  }
  grep -Fq "<h2>$expected_title</h2>" <<<"$chapter" || {
    echo "$name first chapter does not preserve the complete title heading" >&2
    return 1
  }
  grep -Fq ">$expected_title</a>" <<<"$navigation" || {
    echo "$name table of contents has the wrong first chapter title" >&2
    return 1
  }
  grep -Fq "$first_page_marker" <<<"$chapter" || {
    echo "$name first chapter lost expected byline text: $first_page_marker" >&2
    return 1
  }

  local chapter_count
  chapter_count="$(archive_listing "$epub" | grep -Ec '^OEBPS/text/page-[0-9]+\.xhtml$')"
  [[ "$chapter_count" == "$expected_pages" ]] || {
    echo "$name has $chapter_count chapters; expected $expected_pages" >&2
    return 1
  }

  local marker_found=false
  while IFS= read -r member; do
    local body
    body="$(archive_member "$epub" "$member")"
    if grep -Fqi "$expected_marker" <<<"$body"; then
      marker_found=true
      break
    fi
  done < <(archive_listing "$epub" | grep -E '^OEBPS/text/page-[0-9]+\.xhtml$')
  [[ "$marker_found" == true ]] || {
    echo "$name is missing expected text: $expected_marker" >&2
    return 1
  }

  case "$name" in
    quick)
      ! grep -Fq '<h2>System in CloudKit</h2>' <<<"$chapter" || {
        echo "QuiCK retains a detached title fragment" >&2
        return 1
      }
      assert_headings "$epub" OEBPS/text/page-0002.xhtml \
        "At-least-once" "High Availability" "Low overhead" "Scalability"
      assert_headings "$epub" OEBPS/text/page-0003.xhtml \
        "Fairness" "Operations and monitoring"
      assert_headings "$epub" OEBPS/text/page-0004.xhtml \
        "4 FOUNDATIONDB, THE RECORD LAYER AND CLOUDKIT"
      assert_headings "$epub" OEBPS/text/page-0006.xhtml \
        "Push notifications" "Enqueue" "High-level scheduling algorithm"
      assert_headings "$epub" OEBPS/text/page-0007.xhtml \
        "Concurrency between consumers, fairness and leases"
      assert_headings "$epub" OEBPS/text/page-0008.xhtml \
        "Reducing contention between producers and consumers" "Pointer garbage-collection"
      assert_headings "$epub" OEBPS/text/page-0009.xhtml \
        "Correctness" "Isolation level" "User-move and local work items" \
        "6.1 Supporting External Data Stores"
      assert_headings "$epub" OEBPS/text/page-0011.xhtml \
        "Environment and Workload Generation" "Scalability" "Fairness"
      local foundation_chapter
      foundation_chapter="$(archive_member "$epub" OEBPS/text/page-0004.xhtml)"
      ! grep -Fq '<strong>RECORD</strong> <strong>LAYER</strong>' <<<"$foundation_chapter" || {
        echo "QuiCK retains a detached Section 4 title fragment" >&2
        return 1
      }
      local closing_chapter
      closing_chapter="$(archive_member "$epub" OEBPS/text/page-0012.xhtml)"
      grep -Fq $'<h2>9 CONCLUSIONS</h2>\n<p>When queues are needed' <<<"$closing_chapter" || {
        echo "QuiCK conclusions remain joined to the first body paragraph" >&2
        return 1
      }
      grep -Fq $'<h2>ACKNOWLEDGMENTS</h2>\n<p>We thank the past' <<<"$closing_chapter" || {
        echo "QuiCK acknowledgments remain joined to the first body paragraph" >&2
        return 1
      }
      ! archive_member "$epub" OEBPS/text/page-0006.xhtml | grep -Fq '<h2>A Manager thread' || {
        echo "QuiCK body prose was promoted as a heading" >&2
        return 1
      }
      ;;
    attention)
      assert_headings "$epub" OEBPS/text/page-0003.xhtml "Encoder" "Decoder"
      assert_headings "$epub" OEBPS/text/page-0008.xhtml "Residual Dropout" "Label Smoothing"
      assert_headings "$epub" OEBPS/text/page-0010.xhtml "Acknowledgements"
      ! archive_member "$epub" OEBPS/text/page-0010.xhtml | grep -Fq '<h2>WSJ 23 F1</h2>' || {
        echo "Attention table header was promoted as a heading" >&2
        return 1
      }
      ! grep -Fq '>WSJ 23 F1</a>' <<<"$navigation" || {
        echo "Attention table header leaked into navigation" >&2
        return 1
      }
      ! archive_member "$epub" OEBPS/text/page-0011.xhtml | grep -Fq '<h2>[' || {
        echo "Attention reference text was promoted as a heading" >&2
        return 1
      }
      ;;
    bert)
      ! grep -Fq '<h3>Language Understanding</h3>' <<<"$chapter" || {
        echo "BERT retains a detached title fragment" >&2
        return 1
      }
      ! grep -Fq '<h3>Bidirectional Transformers for</h3>' <<<"$chapter" || {
        echo "BERT retains a detached title fragment" >&2
        return 1
      }
      assert_headings "$epub" OEBPS/text/page-0003.xhtml "Model Architecture"
      assert_headings "$epub" OEBPS/text/page-0004.xhtml \
        "Input/Output Representations" "Task #1: Masked LM" \
        "Task #2: Next Sentence Prediction (NSP)"
      assert_headings "$epub" OEBPS/text/page-0012.xhtml \
        "A Additional Details for BERT" "A.1 Illustration of the Pre-training Tasks" \
        "Masked LM and the Masking Procedure"
      assert_headings "$epub" OEBPS/text/page-0014.xhtml \
        "A.4 Comparison of BERT, ELMo ,and OpenAI GPT" \
        "B.1 Detailed Descriptions for the GLUE Benchmark Experiments."
      assert_headings "$epub" OEBPS/text/page-0016.xhtml \
        "C Additional Ablation Studies" "C.2 Ablation for Different Masking Procedures"
      local references_chapter
      references_chapter="$(archive_member "$epub" OEBPS/text/page-0010.xhtml)"
      ! grep -Fq '<h2>Image Database' <<<"$references_chapter" || {
        echo "BERT reference text was promoted as a heading" >&2
        return 1
      }
      ! grep -Fq '<h2>Class-based n-gram models of natural language</h2>' \
        <<<"$references_chapter" || {
        echo "BERT citation title was promoted as a heading" >&2
        return 1
      }
      ;;
    bitcoin)
      assert_headings "$epub" OEBPS/text/page-0001.xhtml "Abstract"
      ;;
  esac
}

if (( $# == 0 )); then
  output_dir="$(mktemp -d "${TMPDIR:-/tmp}/paprika-paper-output.XXXXXX")"
  trap 'rm -rf "$output_dir"' EXIT
  outputs=()
  for index in "${!fixture_names[@]}"; do
    name="${fixture_names[$index]}"
    title="${fixture_titles[$index]}"
    fixture="$(download_fixture \
      "$name" \
      "${fixture_urls[$index]}" \
      "${fixture_sha256s[$index]}")"
    output="$output_dir/$name.paprika.epub"
    cargo run --locked --release --bin paprika -- \
      "$fixture" --output "$output" --title "$title"
    assert_paper \
      "$name" \
      "$output" \
      "$title" \
      "${fixture_pages[$index]}" \
      "${fixture_markers[$index]}" \
      "${fixture_first_page_markers[$index]}"
    outputs+=("$output")

    if [[ "$name" == quick ]]; then
      no_images_output="$output_dir/$name-no-images.paprika.epub"
      cargo run --locked --release --bin paprika -- \
        "$fixture" --output "$no_images_output" --title "$title" --no-images
      assert_paper \
        "$name" \
        "$no_images_output" \
        "$title" \
        "${fixture_pages[$index]}" \
        "${fixture_markers[$index]}" \
        "${fixture_first_page_markers[$index]}"
      image_count="$(archive_listing "$no_images_output" | grep -Ec '^OEBPS/images/' || :)"
      [[ "$image_count" == 0 ]] || {
        echo "QuiCK --no-images output unexpectedly contains image assets" >&2
        exit 1
      }
      outputs+=("$no_images_output")
    fi
  done
  set -- "${outputs[@]}"
fi

for epub in "$@"; do
  [[ -f "$epub" ]] || {
    echo "EPUB does not exist: $epub" >&2
    exit 1
  }
  java -jar "$epubcheck_jar" "$epub"
done
