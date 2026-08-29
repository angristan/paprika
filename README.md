# Paprika

**[Convert a PDF in your browser →](https://paprika.stanislas.cloud)**

Paprika turns born-digital PDFs into compact, reflowable EPUB 3 books for e-readers and phones. Text stays selectable, font size is controlled by the reader, and the document never leaves your device.

The same Rust pipeline runs in the native CLI and browser WebAssembly app. An experimental raster PDF mode remains available for scans and layouts that semantic extraction cannot reconstruct safely.

> Paprika is an early clean-room implementation inspired by the workflow of [k2pdfopt](https://www.willus.com/k2pdfopt/). It does not aim for command-line or output compatibility.

## What works

- Reflowable EPUB 3 output with selectable text
- Column-aware reading order for born-digital PDFs
- Headings, lists, links, basic tables, Unicode inline math, embedded images, captioned vector-figure crops, conservative display-equation crops, and visual column fallbacks for math-dense pages
- One source-page chapter per EPUB spine entry for traceability
- Native CLI on Linux, macOS, and Windows
- Browser conversion in a cancellable Web Worker
- Static Cloudflare deployment with no upload endpoint
- Experimental raster `fit-width`, `fit-page`, and graphical `reflow` PDF output

Semantic PDF extraction is inherently heuristic. Paprika preserves confidently detected display equations as local image crops and keeps simpler inline math as Unicode text; it does not reconstruct LaTeX or MathML. When a page contains too many fragmented mathematical glyphs for trustworthy semantic extraction, Paprika preserves its columns visually instead of emitting corrupted formulas. Uncaptioned vector graphics, unusual layouts, and reading order can require manual review. Paprika does not run OCR; scanned pages are reported and preserved only when their raster image can be extracted. Password-protected PDFs are not supported.

## CLI

Build and convert to EPUB (the default):

```bash
cargo build --release --bin paprika
./target/release/paprika paper.pdf
```

The default output is `paper.paprika.epub` next to the source file.

```bash
# Set book metadata and omit embedded raster images.
paprika paper.pdf --title "Paper title" --language en --no-images

# Experimental raster fallback. A .pdf output path also selects this format.
paprika paper.pdf --format pdf --mode fit-width
paprika paper.pdf --output paper.paprika.pdf
```

Run `paprika --help` for the complete option list. Existing output is never replaced unless `--force` is supplied.

## Browser app

Prerequisites:

- Rust 1.92 or newer with the `wasm32-unknown-unknown` target
- `wasm-pack` 0.15 or newer
- Bun 1.4 or newer

On Arch Linux:

```bash
sudo pacman -S wasm-pack
rustup target add wasm32-unknown-unknown
bun install --frozen-lockfile
bun run dev
```

`bun run build` writes the deployable static site to `web/dist/`. The browser accepts PDFs up to 64 MiB and 500 pages. EPUB image resources are capped at 56 MiB, semantic XHTML at 24 MiB, and final EPUB output at 96 MiB. Raster mode has separate page and working-memory limits. Conversion is synchronous inside a Web Worker, so **Cancel** terminates the worker without blocking the interface.

## Cloudflare

[`wrangler.jsonc`](wrangler.jsonc) defines a static-only Worker at [`paprika.stanislas.cloud`](https://paprika.stanislas.cloud). No Worker script, binding, secret, or storage resource is required.

Cloudflare Workers Builds watches `main`:

- Build: `bun run build:cloudflare`
- Deploy: `bun run wrangler deploy`

The build bootstrap installs pinned Rust and wasm-pack versions and verifies the wasm-pack archive checksum. Release builds use Rust's size optimization but skip the costly final `wasm-opt` pass; set `PAPRIKA_WASM_OPT=1` only when the smallest possible bundle is worth the extra build time.

## Architecture

```text
PDF bytes ──▶ pdf_oxide ──▶ Markdown semantic flow ──▶ XHTML pages
                    │                                      │
                    └──────── embedded raster images ──────┤
                                                           ▼
                                                     EPUB 3 archive
                                                           │
                                             ┌─────────────┴─────────────┐
                                             ▼                           ▼
                                       paprika CLI                 paprika-wasm
                                       filesystem                   Web Worker

PDF bytes ──▶ Hayro rasterizer ──▶ paprika-core layout ──▶ raster PDF fallback
```

- [`paprika-epub`](crates/paprika-epub) extracts semantic content and writes EPUB 3 entirely in memory.
- [`paprika-core`](crates/paprika-core) contains target-independent raster analysis and pagination.
- [`paprika-pdf`](crates/paprika-pdf) renders source pages and writes the raster fallback.
- [`paprika-cli`](crates/paprika-cli) owns filesystem behavior and overwrite safety.
- [`paprika-wasm`](crates/paprika-wasm) owns browser limits and JavaScript bindings.
- [`web`](web) is a dependency-light static interface.

## Development

```bash
bun install --frozen-lockfile
bun run check
```

Rust-only validation:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p paprika-wasm --target wasm32-unknown-unknown
```

For a representative multi-column regression, convert [`QuiCK.pdf`](https://www.foundationdb.org/files/QuiCK.pdf) and verify that Algorithm 2 precedes Algorithm 3 in source page 8, all chapter XHTML remains parseable, and output stays well below the source PDF size.

## License

Paprika is available under either the Apache License 2.0 or the MIT License, at your option.
