# Paprika

Paprika repaginates PDFs for e-readers and phones. It trims margins, detects one- or two-column reading order, and wraps rasterized words onto a device-sized page.

The same Rust pipeline runs as a native CLI and as WebAssembly in a local-only browser app. The static website can be hosted with Cloudflare Workers Static Assets; documents are not uploaded.

> Paprika is an early clean-room implementation inspired by the workflow of [k2pdfopt](https://www.willus.com/k2pdfopt/). It does not aim for command-line or output compatibility.

## What works

- Pure-Rust PDF input and output; no PDFium, Poppler, or system library
- Whitespace trimming
- One- and two-column detection
- Graphical word wrapping for scanned and ordinary PDFs
- `fit-width` and `fit-page` modes
- Configurable output dimensions, DPI, margins, text scale, and white threshold
- Native CLI on Linux, macOS, and Windows
- Browser conversion in a cancellable Web Worker
- Static Cloudflare deployment with no application backend

Paprika currently emits raster PDFs without selectable text. It does not support OCR, encrypted PDFs, DjVu, semantic table reconstruction, deskew/dewarp, right-to-left reading order, or native/vector-preserving output. Complex formulas, diagrams, and tables usually work better with `fit-width` than `reflow`.

## CLI

Build and convert a document:

```bash
cargo build --release --bin paprika
./target/release/paprika paper.pdf
```

The default output is `paper.paprika.pdf` at 758 × 1024 px. Useful alternatives:

```bash
# Keep each trimmed source page intact.
paprika paper.pdf --mode fit-page --width 1072 --height 1448 --dpi 300

# Trim, fit to width, and split tall content across output pages.
paprika paper.pdf --mode fit-width --width 1080 --height 1440 --dpi 265

# Tune graphical reflow.
paprika paper.pdf --font-size 14 --columns 2 --source-dpi 180
```

Run `paprika --help` for the complete option list. Existing output is never replaced unless `--force` is supplied.

## Browser app

Prerequisites:

- Rust 1.92 or newer with the `wasm32-unknown-unknown` target
- `wasm-pack` 0.15 or newer
- Bun 1.4 or newer

On Arch Linux, install `wasm-pack` from the official repository:

```bash
sudo pacman -S wasm-pack
rustup target add wasm32-unknown-unknown
bun install --frozen-lockfile
bun run dev
```

`bun run build` writes the deployable static site to `web/dist/`. Generated WASM and static output are ignored by source control.

The browser build limits input to 64 MiB and 500 pages, each rendered source page to 24 megapixels, and retained output to 32 megapixels. Conversion is synchronous inside a Web Worker, so **Cancel** terminates the worker without blocking the interface. The source and output live only in browser memory.

## Cloudflare

[`wrangler.jsonc`](wrangler.jsonc) defines a static-only Worker. No Worker script, binding, secret, or storage resource is required.

Validate without a remote change:

```bash
bun run check
```

Deploy to the configured Cloudflare account only after reviewing the Worker name and account context:

```bash
bun run deploy
```

## Architecture

```text
                         ┌──────────────────┐
PDF bytes ──────────────▶│ paprika-pdf      │
                         │ Hayro rasterizer │
                         └────────┬─────────┘
                                  │ one RGB source page
                                  ▼
                         ┌──────────────────┐
                         │ paprika-core     │
                         │ trim → regions   │
                         │ → columns → rows │
                         │ → words → pages  │
                         └────────┬─────────┘
                                  │ device-sized RGB pages
                                  ▼
                         ┌──────────────────┐
                         │ pdf-writer       │
                         │ compressed PDF   │
                         └────────┬─────────┘
                                  │
                   ┌──────────────┴──────────────┐
                   ▼                             ▼
             paprika CLI                  paprika-wasm
             filesystem                   Uint8Array + Web Worker
```

- [`paprika-core`](crates/paprika-core) contains target-independent image analysis and pagination.
- [`paprika-pdf`](crates/paprika-pdf) renders source pages and writes raster PDF output.
- [`paprika-cli`](crates/paprika-cli) owns filesystem behavior and overwrite safety.
- [`paprika-wasm`](crates/paprika-wasm) owns browser limits and JavaScript bindings.
- [`web`](web) is a dependency-light static interface.

The optimizer consumes one source raster at a time. It retains completed output pages because the final PDF is returned as one byte buffer; this is the main memory limit for long or high-resolution documents.

## Development

Run the complete validation path:

```bash
bun install --frozen-lockfile
bun run check
```

Rust-only development is faster:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Behavioral tests use synthetic page layouts and a PDF render/write round trip. Browser checks should cover both narrow and wide viewports, file selection, cancel, successful download, and accessibility.

## License

Paprika is available under either the Apache License 2.0 or the MIT License, at your option.
