# Paprika interface contract

Paprika is a local document tool, not a product landing page. The interface helps a reader select one PDF, understand the limits, choose an output, run the job, inspect the result, and download it. Every visible element must support that sequence.

## Product model

- **Audience:** people making papers and documents readable on small screens.
- **Primary job:** convert one local born-digital PDF to a compact, reflowable EPUB.
- **Fallback:** offer raster PDF as an explicit experimental option for scans or layouts that semantic extraction cannot preserve.
- **Active scope:** the file selected in the current browser tab.
- **Consequential actions:** local conversion and output download. Neither changes the source file.
- **States:** empty, source ready, processing, output ready, failed, and canceled.
- **Evidence:** show file identity, output format, progress, limits, warnings, preview state, and download size. Do not invent quality scores, speed claims, visitor counters, or success metrics.

## Creative brief: Paprika Web Edition

```text
Audience · readers turning papers into pocket-sized books
Job · select one PDF, reflow it locally, inspect it, download it
Primary action · Make EPUB
Real content · source identity, limits, format, preview, warnings, result counts
Constraints · local-only processing, 320 px, 200% zoom, keyboard/touch, reduced motion
Objects · source PDF, local conversion job, generated EPUB or raster PDF
Active scope · one browser tab and one selected file
Critical states · empty, ready, working, success, error, canceled
Consequential actions · conversion and download; the source is never changed
Feel · useful, literate, modest, handmade
Avoid · faux application chrome, visual noise, modern SaaS polish, nostalgia props
```

Paprika should feel like a well-made publishing utility on the 1997 web, not a simulated operating system. A narrow white document page carries a serif masthead, pixel pepper mark, blue links, thin rules, and ordinary form controls. The period reference comes from early-web typography and structure rather than decorative clutter.

## Task hierarchy

1. Identify Paprika and its PDF-to-EPUB purpose in the page masthead.
2. Show source, local composition, and result as one quiet text route.
3. Put source selection, limits, output settings, and the primary action in the left column.
4. Keep advanced raster controls hidden until raster output is selected.
5. Keep preview, live status, report, warnings, and download together in the right column.
6. After conversion, retain source identity and a visible **Edit** action without competing with the result.
7. Put the concise privacy and OCR statement below the page.
8. At 320 px and 200% zoom, stack route, source, local divider, and preview in DOM order without horizontal scrolling.

## Visual system

### Type

- Application UI: `Arial, Helvetica, sans-serif`, regular and bold.
- Masthead, page headings, fieldset legends, and report headings: `Georgia, Times New Roman, serif`.
- Machine values and compact route labels: `Courier New, Courier, monospace`.
- Generated EPUB content keeps its document typography inside the sandboxed preview.
- Diagram labels may use 7–10 px. Functional UI uses 11–15 px. The task statement uses a documented fluid 30–39 px range.

### Color

- `--page: #ffffff`, `--surround: #dedede`, and `--soft: #f2f2ee` form the neutral page and preview field.
- `--ink: #111111`, `--muted: #555555`, and `--rule: #9a9a9a` carry text and structure.
- `--blue: #000080` and `--link: #0000cc` identify headings, links, selected state, and trusted structure.
- `--paprika: #a32900` and `--paprika-dark: #751e00` identify the pepper mark, masthead, primary action, and reflowed output edge.
- `--green: #006600`, `--error: #8b0000`, and `--warning: #765000` carry semantic state.

Do not add decorative colors. Color never carries state alone; labels, borders, and position must also communicate it. Static edition and privacy metadata stay plain text; they never use status dots or health indicators.

### Shape and depth

- All surfaces are square.
- Thin solid and dotted rules establish grouping.
- Native-looking raised buttons are the only beveled elements.
- Do not use a desktop wallpaper, application-window frame, stacked title bars, patterns, gradients, soft shadows, glass, glow, blur, rounded cards, or floating panels.
- Do not box every datum. Source, preview, report, and footer remain part of one document page.

### Spacing

- Dense form groups use 5–12 px gaps.
- Major regions use 18–28 px gaps.
- The desktop first viewport must expose the file action and preview without marketing whitespace.

### Signature

The pixel pepper mark and the simple fixed-page-to-reflowed-page diagram carry the identity. The narrow **Local** divider shows three restrained packets only while conversion is running. Reduced-motion mode removes packet movement without removing status.

## Deliberate 1990s references

- The centered white page, serif masthead, pixel mark, default blue links, dotted rules, native controls, fieldsets, and compact text route are intentional.
- The interface must not add fake browser or operating-system chrome, patterned backgrounds, rainbow accents, fake visitor counters, awards, web rings, “under construction” warnings, marquees, blinking body text, cursor trails, autoplay, dead navigation, or external image requests.
- Nostalgia never overrides hierarchy, contrast, touch size, error recovery, or the conversion flow.

## Interaction and accessibility

- Keep all controls keyboard operable with a visible 3 px navy focus outline.
- Use native controls and semantic landmarks before custom widgets.
- Keep labels visible; placeholders are not labels.
- Status changes remain in the polite live region. Errors use text and geometry, never color alone.
- Primary controls remain at least 44 px high. Supporting text remains at least 11 px.
- Source and generated previews identify the active document and trust boundary.
- A successful result retains source identity; **Edit** restores every setting without discarding the output.
- Cancellation keeps the source selected and states what was discarded.
- The layout must not overflow at 320 CSS pixels or 200% zoom.
- Touch and keyboard users receive every action that hover users receive.

## Privacy contract

Document bytes stay in the browser tab and Web Worker. The deployed app has no application server, upload route, analytics, storage binding, or remote conversion service. Runtime code must not add third-party requests. Privacy copy must still distinguish document processing from ordinary requests for the static site: Cloudflare and the user’s network can observe normal HTTP metadata when the app assets are loaded. See [`docs/privacy.md`](docs/privacy.md).
