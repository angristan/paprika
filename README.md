# Paprika

**Make PDFs readable on small screens—without sending document bytes to a server.**

[Open Paprika in your browser](https://paprika.stanislas.cloud)

![Paprika converting a paper into a reflowable EPUB](docs/screenshots/desktop-result.png)

PDFs preserve pages. E-readers and phones need reading flow. Paprika rebuilds born-digital PDFs as compact EPUB 3 books with selectable text, reader-controlled typography, and local previews.

The browser app and native CLI use the same Rust conversion pipeline. Browser conversion runs in a Web Worker; Cloudflare serves static application files but does not receive the selected document.

> Paprika is an early clean-room project inspired by the workflow of [k2pdfopt](https://www.willus.com/k2pdfopt/). It does not provide k2pdfopt command-line or output compatibility.

## Output guide

| Output | Best for | Trade-off |
| --- | --- | --- |
| **EPUB 3** · default | Born-digital papers, reports, and books | Reflowable and selectable; complex layout is reconstructed heuristically |
| **Raster PDF** · experimental | Scans or layouts unsuitable for semantic extraction | Image-only and not text-selectable; fit modes retain page geometry while reflow rearranges content |

Paprika does not run OCR. A scanned document needs OCR before it can become a semantic EPUB.

## Use Paprika

### Browser

Open [paprika.stanislas.cloud](https://paprika.stanislas.cloud), choose a PDF, and select **Make EPUB**. Review the generated book and any warnings before downloading it.

Document bytes stay in the tab. Normal requests for HTML, JavaScript, WebAssembly, and other static assets still reach Cloudflare. See the full [privacy and preview security model](docs/privacy.md).

Browser input is limited to 64 MiB and 500 pages. The preview is intentionally smaller than the complete downloadable book.

### Command line

The CLI is tested on Linux, macOS, and Windows. Build and install it from this checkout:

```bash
cargo install --locked --path crates/paprika-cli
```

Convert to EPUB, the default output:

```bash
paprika paper.pdf
# writes paper.paprika.epub
```

Set book metadata or omit embedded raster images:

```bash
paprika paper.pdf --title "Paper title" --language en
paprika paper.pdf --no-images
```

Use an experimental raster fallback:

```bash
paprika paper.pdf --format pdf --mode fit-width
paprika paper.pdf --format pdf --mode fit-page
paprika paper.pdf --format pdf --mode reflow
```

A `.pdf` destination also selects raster output:

```bash
paprika paper.pdf --output paper.paprika.pdf
```

Paprika never replaces an existing destination unless `--force` is supplied. Run `paprika --help` for all raster dimensions, resolution, margin, threshold, and column options.

## What survives conversion

Paprika preserves content semantically when the PDF provides enough reliable information:

- selectable text and reading order;
- headings, lists, links, and basic tables;
- embedded images and captioned vector figures;
- simple inline mathematics as Unicode text;
- one EPUB chapter per source page for traceability.

When semantic extraction would corrupt a figure or equation, Paprika can use a bounded local image crop instead. Formula-heavy pages can fall back to visual columns while trustworthy prose remains selectable. These visual fallbacks require image preservation and are skipped for source files over 32 MiB. Paprika does not guess LaTeX or authoritative MathML.

Always review the output. Unusual page geometry, uncaptioned graphics, dense tables, and fragmented fonts can still produce imperfect reading order. Password-protected PDFs are not supported.

## How it works

```text
PDF bytes
├── pdf_oxide ── text, structure, links, embedded images ─┐
└── Hayro ───── bounded figure, equation, and page crops ─┼──▶ EPUB 3
                                                          │
                                                          ├──▶ native CLI
                                                          └──▶ browser Web Worker

PDF bytes ──▶ Hayro ──▶ paprika-core layout ──▶ raster PDF fallback
```

The conversion pipeline enforces explicit limits on source pages, decoded pixels, working canvases, embedded assets, previews, semantic output, and final archives. The CLI writes through a sibling staging file and installs the result atomically.

| Path | Responsibility |
| --- | --- |
| [`paprika-epub`](crates/paprika-epub) | Semantic extraction, visual fallbacks, XHTML, and EPUB packaging |
| [`paprika-core`](crates/paprika-core) | Target-independent raster analysis and pagination |
| [`paprika-pdf`](crates/paprika-pdf) | Source rendering and raster PDF output |
| [`paprika-cli`](crates/paprika-cli) | Native arguments, filesystem safety, and reporting |
| [`paprika-wasm`](crates/paprika-wasm) | Browser limits and WebAssembly bindings |
| [`web`](web) | Dependency-light static interface and previews |

## Develop

Requirements:

- Rust 1.97.1 with `wasm32-unknown-unknown`;
- wasm-pack 0.15.0;
- Bun 1.4.0;
- Java 17 or newer for EPUBCheck validation.

Install wasm-pack 0.15.0 with your platform package manager, then prepare the pinned Rust target and JavaScript dependencies:

```bash
rustup target add wasm32-unknown-unknown
bun install --frozen-lockfile
bun run dev
```

`bun run dev` builds the WebAssembly package and starts a local Wrangler server. `PAPRIKA_WASM_OPT=1 bun run build` enables the slower optional final size optimization.

Run the release gates before submitting a change:

```bash
bun run check       # Rust, WebAssembly, JavaScript, and fuzz-target checks
bun run predeploy   # EPUBCheck, production build, distribution checks, dry run
bun run playwright install chromium firefox webkit
bun run test:e2e    # Chromium, Firefox, and WebKit against loopback Wrangler
```

The browser suite never targets production. Cloudflare Workers Builds deploys accepted `main` commits only after the protected GitHub checks pass. See the [audit evidence](docs/audit-validation.md) and [release, smoke-test, and rollback procedure](docs/release.md).

## License

Paprika is available under either the [Apache License 2.0](LICENSE-APACHE) or the [MIT License](LICENSE-MIT), at your option.
