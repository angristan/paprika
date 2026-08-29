# Reliability and redesign audit

This document records the acceptance evidence for the reliability and interface audit implemented on `feat/reliability-redesign`. Product-code validation was completed at `c77ef254485d0c37d8f1e6fd8cb30af490d1eba6`; this document and its screenshots do not change the deployed application.

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

GitHub Actions run [33279349031](https://github.com/angristan/paprika/actions/runs/33279349031) passed the locked build job and native tests on Ubuntu 24.04, macOS 14, and Windows 2022. Its browser step passed all 39 tests across Chromium, Firefox, and WebKit. Each engine ran the axe WCAG A/AA audit, local-only request check, 320 CSS-pixel layout check, 200% zoom check, EPUB conversion/download, warning, cancellation/retry, raster preview, and diagnostic-recovery cases.

`main` branch protection requires these four exact checks with strict status checks and administrator enforcement:

- `Locked build and smoke tests`
- `Native tests (ubuntu-24.04)`
- `Native tests (macos-14)`
- `Native tests (windows-2022)`

## Reproducible build and supply chain

Two consecutive executions of `scripts/build-web-cloudflare.sh` produced the same aggregate SHA-256 over every file in `web/dist/`:

```text
031e8e956fc4e241b702b85aa4aa4d789e0a97e4c530a92b66d570a7c6bcefa1
```

The first execution populated the cache. The second finished without Rust compilation and restored verified WASM cache entry:

```text
d3b2cfd44c6ef0f106df84c687a01222388de9c701e292535090899bbba6915a
```

The build scripts verify pinned SHA-256 values before executing or consuming downloaded rustup-init, wasm-pack archive/binary, EPUBCheck archive/JAR, and the QuiCK regression fixture. Cached WASM files have their own `SHA256SUMS`, and the cache key covers the pinned compiler, wasm-pack version, optimization mode, Rust flags, lockfile, manifests, Rust sources, and build script.

## Rendered review

The following screenshots were captured from a fresh local `web/dist/` build at the validated product commit using Chromium and the public QuiCK PDF fixture.

### Desktop, empty state

![Desktop empty state](screenshots/desktop-empty.png)

The first viewport has one clear conversion task, a compact source-to-result route, adjacent controls and preview, and visible privacy context. The design uses typography, rules, and restrained Paprika red instead of gradients, glass, floating cards, fake metrics, feature tiles, or oversized marketing copy.

### Desktop, completed EPUB

![Desktop completed EPUB](screenshots/desktop-result.png)

The result keeps the source controls available, identifies the active preview, shows selectable book content, discloses preview truncation, reports source/text/image counts, presents warnings before download, and gives the download one clear visual priority.

### Mobile, 320 CSS pixels

![Mobile result at 320 CSS pixels](screenshots/mobile-result-320px.png)

The workflow becomes one column without horizontal scrolling. Controls retain visible labels and usable target sizes; status, preview navigation, report, warnings, download, limitations, and privacy remain in document order.

### 200% zoom

![Result at 200 percent zoom](screenshots/zoom-200-percent.png)

At 200% zoom the same task order remains usable without horizontal document overflow. Controls, preview, report, download, limitations, and privacy stay reachable. Text wraps or truncates only inside explicitly bounded compact route/control labels.

The automated axe scan found no WCAG 2 A/AA or WCAG 2.1 A/AA violations in Chromium, Firefox, or WebKit. The CSS also provides visible `:focus-visible` treatment, at least 44-pixel primary control targets, semantic native controls, live status regions, and a reduced-motion override.

## Documentation review

- `README.md` covers CLI behavior, browser limits and lifecycle, preview behavior, Cloudflare deployment, architecture, local development, and each validation command.
- `docs/privacy.md` distinguishes local document processing from normal static-host request metadata, explains object-URL retention limits, and documents the separate EPUB-sandbox and browser-native PDF trust boundaries.
- `docs/release.md` defines the locked release gate, required GitHub checks, pinned external inputs, exact Workers Builds settings, production smoke checks, and an approval-gated non-destructive rollback procedure.
- `DESIGN.md` defines the task-first print-workshop direction, evidence requirements, 320-pixel behavior, anti-pattern exclusions, keyboard/touch expectations, preview state, and reduced-motion behavior.

No production deployment or merge is part of this audit branch.