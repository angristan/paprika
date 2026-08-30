# Reliability and redesign audit

This document records the acceptance evidence for the reliability and interface audit implemented on `feat/reliability-redesign`. The branch remains separate from `main`; this document and its screenshots do not describe a production deployment.

## Defect-to-test traceability

| Risk | Implemented safeguard | Defect-detecting evidence |
| --- | --- | --- |
| Failed CLI writes truncate an existing destination | Write to a sibling temporary file, flush it, then install it atomically | `failed_forced_write_does_not_truncate_existing_destination` injects a partial-write failure and verifies that the original bytes remain and no staging file leaks |
| A no-clobber write races with another writer | Install completed staging files without replacing an existing destination | `no_force_write_does_not_clobber_a_racing_destination` creates the destination during the staged write and verifies that the competing bytes win |
| Raster pages silently clip writes | Reject source or destination rectangles outside their raster bounds | `out_of_bounds_blits_fail_without_modifying_the_destination` verifies both an error and an unchanged destination |
| Oversized content can exceed the target canvas | Scale to both available width and height | `oversized_words_fit_inside_both_canvas_dimensions` verifies the painted rows stop inside the usable canvas |
| Raster memory accounting omits the active output canvas | Charge the active canvas and release completed-page budget when pages are drained | `output_pixel_budget_counts_the_active_canvas`, `draining_completed_pages_releases_pixel_budget`, and `streams_documents_larger_than_the_raster_buffer_budget` |
| Encoded images or the final EPUB can exceed output limits during writes | Use bounded writers for image encoders and the EPUB archive | `bounds_encoded_image_buffers` and `bounds_final_epub_writes` force small limits and require the matching limit errors |
| XHTML wrappers or image decoding bypass semantic budgets | Account final rendered XHTML plus per-page and cumulative image objects/pixels | `accounts_for_the_complete_rendered_xhtml` and `enforces_per_page_and_cumulative_image_limits` |
| Recoverable scan pages are silently omitted | Recognize full-page images and downsample within the pixel budget | `downsamples_recoverable_full_page_scans` checks downsampling, visual-page classification, and rejection beyond the safe bound; `distinguishes_empty_and_image_only_books` verifies warning classification |
| Caption text alone creates a false figure | Require nearby graphic geometry and tighten semantic exclusion to it | `ignores_figure_captions_without_graphic_geometry`, `tightens_semantic_exclusion_to_graphic_bounds`, and `measures_image_overlap_for_crop_deduplication` |
| Mathematical text is emitted confidently when extraction is unreliable | Keep trustworthy prose, crop bounded visual regions, and reject false equation anchors | Equation, math-density, visual-column, figure/table veto, and prose-retention tests in `crates/paprika-epub/src/tests.rs` |
| Invalid or unsafe document language reaches EPUB XML | Parse and canonicalize BCP 47 tags, falling back to `en` | `rejects_invalid_language_metadata`; a real `--language fr-FR` conversion was also inspected for `<dc:language>fr-FR</dc:language>`, `xml:lang="fr-FR"`, and `lang="fr-FR"` |
| Browser failures are silent or leave stale state | Report sanitized diagnostics, reject stale worker generations, and recreate workers after cancel/failure | Cross-browser tests `cancels a job and converts again with a fresh worker`, `surfaces conversion warnings before download`, and `shows safe diagnostics and recovers from invalid input` |
| Preview security boundaries are conflated | Keep generated EPUB XHTML sanitized and sandboxed; use the documented browser-native boundary for local PDF blobs | Cross-browser test `uses the documented local PDF preview boundary` plus `docs/privacy.md` |
| Narrow and zoomed layouts clip controls | Remove minimum viewport assumptions, bound native controls and fieldsets, and include actionable overflow diagnostics | Cross-browser tests `keeps the conversion workbench usable at 320 CSS pixels` and `remains usable at 200 percent zoom` |

The Rust tests intentionally exercise failures, races, bounds, and observable output rather than mirroring internal call sequences.

## Automated validation

The local release gate was run from the feature worktree:

```text
bun run predeploy
  rustfmt: passed
  Clippy --workspace --all-targets -D warnings: passed
  workspace tests: 57 passed (CLI 4, core 14, EPUB 33, PDF 6)
  wasm32-unknown-unknown check: passed
  browser JavaScript bundle/static-entry checks: passed
  bounded fuzz-target compilation: passed
  QuiCK conversion: 13 source pages, 13 text pages, 10 images
  EPUBCheck 5.3.0: 0 fatals, 0 errors, 0 warnings
  production WASM build: passed
  deploy-file allowlist and license comparison: passed
  wrangler deploy --dry-run: passed
```

The [feature-branch workflow history](https://github.com/angristan/paprika/actions/workflows/pre-deploy.yml?query=branch%3Afeat%2Freliability-redesign) records the locked build job and native tests on Ubuntu 24.04, macOS 14, and Windows 2022. Its browser step runs 39 tests across Chromium, Firefox, and WebKit. Each engine runs the axe WCAG A/AA audit, local-only request check, 320 CSS-pixel layout check, 200% zoom check, EPUB conversion/download, warning, cancellation/retry, raster preview, and diagnostic-recovery cases.

`main` branch protection requires these four exact checks with strict status checks and administrator enforcement:

- `Locked build and smoke tests`
- `Native tests (ubuntu-24.04)`
- `Native tests (macos-14)`
- `Native tests (windows-2022)`

## Reproducible build and supply chain

Two consecutive executions of `scripts/build-web-cloudflare.sh` produced the same aggregate SHA-256 over every file in `web/dist/`:

```text
8af9130303ccd943652ffc406ebbe4874cbdfd1b2dbe04e5e0cb364edbce3b36
```

The first execution populated the cache. The second finished without Rust compilation and restored verified WASM cache entry:

```text
cfdc49986770288878f31fcceedeb4788ed18cb47edcb27238aa3ef754396f21
```

The build scripts verify pinned SHA-256 values before executing or consuming downloaded rustup-init, wasm-pack archive/binary, EPUBCheck archive/JAR, and the QuiCK regression fixture. Cached WASM files have their own `SHA256SUMS`, and the cache key covers the pinned compiler, wasm-pack version, optimization mode, Rust flags, lockfile, manifests, Rust sources, and build script.

## Rendered review

The following screenshots were captured from a fresh local `web/dist/` build at the validated product commit using Chromium and the public QuiCK PDF fixture.

### Desktop, empty state

![Desktop empty state](screenshots/desktop-empty.png)

The first viewport is one open folio: source controls form the rigid left leaf, a Paprika seam carries the transformation, and the cool proofing field forms the flexible reader leaf. The route acts as a running head instead of a separate navigation bar. There are no gradients, glass panels, floating cards, fake metrics, or feature tiles.

### Desktop, completed EPUB

![Desktop completed EPUB](screenshots/desktop-result.png)

The completed state contracts the source leaf into a narrow editable file cover while the reader proof expands. It identifies the active preview, bounds prose to a readable measure, distinguishes the capped preview from complete-download totals, reports source/text/image counts, presents one warning summary before download, and gives the download one clear visual priority.

### Mobile, 320 CSS pixels

![Mobile result at 320 CSS pixels](screenshots/mobile-result-320px.png)

The folio closes into one column without horizontal scrolling. Its reflow seam turns downward between the compact file/format summary and reader proof; **Edit** restores the full labeled controls. Counts become full-width rows instead of compressed cards, and preview navigation, warnings, download, and privacy remain in document order.

### 200% zoom

![Result at 200 percent zoom](screenshots/zoom-200-percent.png)

At 200% zoom the same task order remains usable without horizontal document overflow. Controls, preview, report, download, and privacy stay reachable. Compact route labels remain complete, and longer text wraps without horizontal overflow.

The automated axe scan found no WCAG 2 A/AA or WCAG 2.1 A/AA violations in Chromium, Firefox, or WebKit. The CSS also provides visible `:focus-visible` treatment, at least 44-pixel primary control targets, semantic native controls, live status regions, and a reduced-motion override.

### Impeccable catalogue pass

The rendered states were re-audited against the complete [Impeccable slop catalogue](https://impeccable.style/slop/#catalog). The remediation:

- uses one folio boundary instead of separate cards for route, settings, and proof;
- turns the route into an unboxed running head and contracts the completed source leaf;
- uses the central reflow seam as the only high-color structural element;
- removes duplicate status, helper, limitation, and privacy copy;
- limits EPUB prose measure, preserves 16-pixel narrow-screen gutters, and turns mobile counts into aligned rows; and
- documents every application font, color, radius, type-size role, and intentional exception in `DESIGN.md`.

The central Paprika seam and empty PDF → EPUB sheet transformation are intentional functional diagrams: they show fixed page lines being reflowed into a reader page and animate only during active conversion. The serif text inside the preview belongs to the generated EPUB, not the application type system.

## Documentation review

- `README.md` covers CLI behavior, browser limits and lifecycle, preview behavior, Cloudflare deployment, architecture, local development, and each validation command.
- `docs/privacy.md` distinguishes local document processing from normal static-host request metadata, explains object-URL retention limits, and documents the separate EPUB-sandbox and browser-native PDF trust boundaries.
- `docs/release.md` defines the locked release gate, required GitHub checks, pinned external inputs, exact Workers Builds settings, production smoke checks, and an approval-gated non-destructive rollback procedure.
- `DESIGN.md` defines the reflowing-folio direction, evidence requirements, 320-pixel behavior, anti-pattern exclusions, keyboard/touch expectations, preview state, and reduced-motion behavior.

No production deployment or merge is part of this audit branch.
