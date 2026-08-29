# Paprika

**[Convert a PDF in your browser →](https://paprika.stanislas.cloud)**

Paprika turns born-digital PDFs into compact, reflowable EPUB 3 books for e-readers and phones. Text stays selectable, font size is controlled by the reader, and the browser app does not upload document bytes.

The same Rust pipeline runs in the native CLI and browser WebAssembly app. An experimental raster PDF mode remains available for scans and layouts that semantic extraction cannot reconstruct safely. See the precise [privacy and data-handling boundary](docs/privacy.md).

> Paprika is an early clean-room implementation inspired by the workflow of [k2pdfopt](https://www.willus.com/k2pdfopt/). It does not aim for command-line or output compatibility.

## What works

- Reflowable EPUB 3 output with selectable text
- Column-aware reading order for born-digital PDFs
- Headings, lists, links, basic tables, Unicode inline math, embedded images, captioned vector-figure crops, conservative display-equation crops, and visual column fallbacks for math-dense pages
- One source-page chapter per EPUB spine entry for traceability
- Native CLI on Linux, macOS, and Windows
- Browser conversion in a cancellable, reusable Web Worker
- Local source-PDF previews through the browser PDF viewer and sanitized, sandboxed generated-EPUB previews
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

- Rust 1.97.1 with the `wasm32-unknown-unknown` target (pinned by `rust-toolchain.toml`)
- `wasm-pack` 0.15.0
- Bun 1.4.0

On Arch Linux:

```bash
sudo pacman -S wasm-pack
rustup target add wasm32-unknown-unknown
bun install --frozen-lockfile
bun run dev
```

`bun run build` writes the complete deployable static site, including both license files, to `web/dist/`. The browser accepts PDFs up to 64 MiB and 500 pages. EPUB image resources are capped at 56 MiB, semantic XHTML at 24 MiB, and final EPUB output at 96 MiB. The generated EPUB preview is separately bounded to 12 chapters, 2 MiB of XHTML, 8 MiB of images, and 48 assets; the download always contains the complete result. Raster mode has separate page and working-memory limits. Conversion is synchronous inside a Web Worker, so **Cancel** can terminate it without blocking the interface. Small successive jobs reuse the initialized worker briefly; large or idle jobs recycle it because WebAssembly linear memory cannot shrink.

## Cloudflare

[`wrangler.jsonc`](wrangler.jsonc) defines a static-only Worker at [`paprika.stanislas.cloud`](https://paprika.stanislas.cloud). No Worker script, binding, secret, or storage resource is required.

Cloudflare Workers Builds watches `main`:

- Build: `bun run build:cloudflare`
- Deploy: `bun run deploy`

The deploy command only uploads the already-built `web/dist/`; it does not rebuild. The build bootstrap verifies pinned `rustup-init` and wasm-pack binaries before execution. It keeps Rust/Cargo data in Cloudflare's cached Bun directory and reuses a WASM package only when its complete Rust-input fingerprint and stored checksums match. Release builds use Rust's size optimization but skip the costly final `wasm-opt` pass; set `PAPRIKA_WASM_OPT=1` only when the smallest possible bundle is worth the extra build time.

See [release, smoke-test, and rollback procedures](docs/release.md).

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

All Cargo commands use `Cargo.lock`. The full release gate also requires Java 17 or newer, wasm-pack 0.15.0, and Playwright's Chromium, Firefox, and WebKit engines:

```bash
bun run playwright install chromium firefox webkit
bun run predeploy
bun run test:e2e
```

Focused validation:

```bash
bun run check:rust   # rustfmt, Clippy, and workspace tests
bun run check:wasm   # wasm32 compilation
bun run check:js     # parse and bundle browser modules
bun run check:fuzz   # compile the bounded parser/layout fuzz harnesses
bun run check:epub   # convert a checksum-pinned fixture and run EPUBCheck 5.3.0
```

The GitHub pre-deploy workflow runs the full gate and Chromium, Firefox, and WebKit smoke tests for pull requests and `main`. Browser tests always start a fresh local Wrangler server on `127.0.0.1`; they cannot silently test the production site. See the [reliability audit evidence](docs/audit-validation.md) for defect-to-test traceability and reviewed desktop, mobile, and zoom renderings.

For a representative multi-column regression, convert [`QuiCK.pdf`](https://www.foundationdb.org/files/QuiCK.pdf) and verify that Algorithm 2 precedes Algorithm 3 in source page 8, all chapter XHTML remains parseable, and output stays well below the source PDF size.

## License

Paprika is available under either the Apache License 2.0 or the MIT License, at your option.
