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

## Task-first hierarchy

1. Name the tool and its local PDF-to-EPUB purpose in one compact header.
2. Put file selection and the recommended EPUB action in the first task area.
3. Keep advanced raster controls hidden until raster output is selected.
4. Keep source/output preview and status adjacent to the controls that change them.
5. Put limitations and privacy facts near the task. Do not turn them into marketing sections.
6. At 320 px, preserve the same order without horizontal scrolling or a stretched first viewport.

The initial viewport must not be padded to fill the screen. Content height follows the task. Use compact, deliberate spacing rather than one repeated spacing value everywhere.

## Visual system

- **Color:** neutral white surfaces and graphite text. Paprika red is reserved for the primary action, focus, and meaningful state emphasis. Do not use cream, beige, parchment, sepia, or warm-paper palettes.
- **Type:** use the operating system UI stack for interface and editorial text. Use monospace only for real machine values such as byte counts, dimensions, job identifiers, and technical status. Do not use or imitate Inter. Do not use crushed letter spacing.
- **Scale:** use a moderate heading scale. The page title must read as a task heading, not an oversized hero statement.
- **Shape:** controls and surfaces are square or use at most a 2 px radius. Shape must communicate grouping or affordance, not decoration.
- **Depth:** use borders, spacing, and background contrast. Do not use decorative shadows, floating cards, glass effects, or the common hairline-border plus wide-shadow card treatment.
- **Spacing:** use tighter spacing inside controls and groups, and larger spacing only between task phases. Avoid monotonous spacing and avoid stretching sections to occupy the first viewport.
- **Motion:** only use motion to show real progress or state change. Respect reduced-motion preferences.

## Explicit anti-patterns

Paprika must not use:

- kicker or eyebrow labels above headings;
- oversized hero copy or a landing-page hero composition;
- cream/beige backgrounds or a faux editorial-paper theme;
- crushed tracking, all-caps ornament, or tiny functional text;
- decorative cards, wide shadows, gradients, glass, or glow;
- a colored side-tab, edge stripe, or unrelated accent bar;
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
- Cancellation must leave the selected source intact and state what was discarded.

## Privacy contract

Document bytes stay in the browser tab and Web Worker. The deployed app has no application server, upload route, analytics, storage binding, or remote conversion service. Runtime code must not add third-party requests. Privacy copy must still distinguish document processing from ordinary requests for the static site: Cloudflare and the user's network can observe normal HTTP metadata when the app assets are loaded. See [`docs/privacy.md`](docs/privacy.md).
