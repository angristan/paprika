# Paprika interface contract

Paprika is a local document tool, not a product landing page. The interface must help a reader select one PDF, understand the conversion limits, choose an output, run the job, inspect the result, and download it. Every visible element must support that sequence.

## Product model

- **Audience:** people making papers and documents readable on small screens.
- **Primary job:** convert one local born-digital PDF to a compact, reflowable EPUB.
- **Fallback:** offer raster PDF as an explicit experimental option for scans or layouts that semantic extraction cannot preserve.
- **Active scope:** the file selected in the current browser tab.
- **Consequential actions:** local conversion and output download. Neither changes the source file.
- **States:** empty, source ready, processing, output ready, failed, and canceled.
- **Evidence:** show file identity, output format, progress, limits, warnings, preview state, and download size. Do not invent quality scores, speed claims, or success metrics.

## Creative brief

- **Audience:** people turning papers and documents into books for small screens.
- **Job:** select one local PDF, choose output settings, convert, inspect, and download.
- **Objects:** source leaf, composition settings, reflow state, reader proof, conversion report, and output file.
- **Feel:** energetic, tactile, and precise—like an open folio in motion rather than a control panel.
- **Avoid:** generic dashboard chrome, nostalgic print-shop costume, and decoration unrelated to document transformation.

## Task-first hierarchy

1. Name the tool and its local PDF-to-EPUB purpose in one compact header.
2. Make the workbench one open folio: source controls on the left, a reflow seam in the center, and the reader proof on the right.
3. Keep the ordered route as a running head on the folio rather than three boxed dashboard steps.
4. Keep advanced raster controls hidden until raster output is selected.
5. After conversion, contract the source leaf into a narrow editable file/format cover while the proof expands.
6. Keep source/output preview, status, report, and download adjacent inside the result leaf.
7. Put limitations and privacy facts near the task. Do not turn them into marketing sections.
8. At 320 px and 200% zoom, stack source, horizontal reflow seam, and proof in DOM order without horizontal scrolling.

The initial viewport must not be padded to fill the screen. Content height follows the task. Use compact, deliberate spacing rather than one repeated spacing value everywhere.

## Visual system

- **Color:** use `#ededed` as the surrounding desk and `#ffffff` as the folio. The remaining neutral scale is `#f1f1f1`, `#dddddd`, `#aaaaaa`, `#767676`, `#737373`, `#666666`, `#5e5e5e`, and `#252525`. Cool proofing surfaces `#f0f7f8` and `#dcebed` are reserved for the document preview. Paprika identity/action colors `#c43d28` and `#942b1d` mark the title outcome, active route, primary action, current selection, and the central reflow seam; success is `#296246` and error is `#a32d21`. Pale `#fff7f5` and `#fff4f1` surfaces are reserved for interactive hover and error feedback. Do not use cream, beige, parchment, sepia, or warm-paper palettes.
- **Type:** use the operating system UI stack for application chrome and explanatory copy. Use monospace only for real machine values such as byte counts, dimensions, job identifiers, and technical status. The generated EPUB preview is document content rather than application chrome and may use the reader-oriented serif/sans-serif styles packaged with the book. Do not use or imitate Inter. Do not use crushed letter spacing.
- **Scale:** core UI text uses 13, 14, 15, 16, 19, and 20 px steps. Twelve pixels is reserved for compact route/header text at narrow effective widths; 17 and 18 px are limited to the mark and compact result values. Task headings use 25 or 30 px at narrow widths and a responsive 30–48 px range on wide screens. The page title must read as a task heading, not an oversized hero statement.
- **Shape:** controls and surfaces are square or use at most a 2 px radius. The workbench has one outer folio boundary; its source and proof leaves are divided by the central seam instead of separate panel boxes. Shape must communicate grouping or affordance, not decoration.
- **Depth:** use the gray desk, white folio, cool proof surface, borders, and spacing. Inner status and report sections use rules rather than nested cards. Do not use decorative shadows, floating cards, glass effects, or the common hairline-border plus wide-shadow card treatment.
- **Spacing:** use tighter spacing inside controls and groups, and larger spacing only between task phases. Avoid monotonous spacing and avoid stretching sections to occupy the first viewport.
- **Motion:** only use motion to show real progress or state change. Respect reduced-motion preferences.
- **Signature:** the whole workbench behaves as a reflowing folio. A narrow Paprika seam carries lines from the rigid source leaf into the flexible reader proof, animating only while conversion is active. The empty-state source/reader diagram repeats that transformation at document scale. This is functional process feedback, not decorative shape art.

## Explicit anti-patterns

Paprika must not use:

- kicker or eyebrow labels above headings;
- oversized hero copy or a landing-page hero composition;
- cream/beige backgrounds or a faux editorial-paper theme;
- crushed tracking, all-caps ornament, or tiny functional text;
- decorative cards, wide shadows, gradients, glass, or glow;
- a colored side-tab, edge stripe, or unrelated accent bar; the central reflow seam is the sole intentional exception because it carries the document transformation;
- feature-card grids, testimonial/metric blocks, or marketing filler;
- the same gap or padding at every hierarchy level;
- a viewport-height shell that pushes the task below the fold;
- monospace for prose, labels, or decoration;
- color without a state or action meaning.

## Interaction and accessibility

- Keep all controls keyboard operable with visible focus.
- Use native controls and semantic landmarks before custom widgets.
- Keep labels visible; placeholders are not labels.
- Status changes must remain in the existing polite live region.
- Do not rely on color alone for error, progress, selection, or completion.
- Touch targets must remain usable without making supporting text tiny.
- Source and generated previews must identify which document is active.
- A successful result must retain source identity in the contracted cover, and **Edit** must restore every setting without discarding the output.
- Cancellation must leave the selected source intact and state what was discarded.

## Privacy contract

Document bytes stay in the browser tab and Web Worker. The deployed app has no application server, upload route, analytics, storage binding, or remote conversion service. Runtime code must not add third-party requests. Privacy copy must still distinguish document processing from ordinary requests for the static site: Cloudflare and the user's network can observe normal HTTP metadata when the app assets are loaded. See [`docs/privacy.md`](docs/privacy.md).
