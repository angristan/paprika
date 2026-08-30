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

## Creative brief: Paprika 97

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
Feel · calm, direct, hand-built, dependable
Avoid · visual noise, modern SaaS polish, illegible nostalgia, fake browser controls
```

Paprika should feel like a dependable desktop utility from 1997, not a collage of 1990s references. One gray application window sits on a solid teal desktop. A single navy title bar establishes identity. Familiar beveled controls and native form elements carry the period character. Paprika red is reserved for the main action and the reflowed edge.

## Task hierarchy

1. Identify Paprika as a local PDF → EPUB utility in the title bar.
2. Show the three real stages—source, local composition, and result—as one quiet text route.
3. Put source selection, limits, output settings, and the primary action in the left pane.
4. Keep advanced raster controls hidden until raster output is selected.
5. Keep preview, live status, report, warnings, and download together in the right pane.
6. After conversion, retain source identity and a visible **Edit** action without competing with the result.
7. Put the concise privacy and OCR statement below the window.
8. At 320 px and 200% zoom, stack route, source, local divider, and preview in DOM order without horizontal scrolling.

## Visual system

### Type

- Application UI: `Tahoma, Verdana, Arial, sans-serif`, regular and bold.
- Machine values and compact route labels: `Courier New, Courier, monospace`.
- Generated EPUB content keeps its document typography inside the sandboxed preview.
- Diagram labels may use 7–10 px. Functional UI uses 11–15 px. The task statement uses a documented fluid 34–48 px range.

### Color

- `--desktop: #008080` is the only page background.
- `--window: #c0c0c0`, `--window-light: #ffffff`, `--window-mid: #808080`, and `--window-dark: #000000` define utility chrome and bevels.
- `--title: #000080` is limited to the title bar, selected state, and trusted structure.
- `--paprika: #a32900` and `--paprika-dark: #751e00` identify the primary action and reflowed output.
- `--success: #006600`, `--error: #8b0000`, `--warning: #7a4b00`, `--paper: #ffffff`, and `--ink: #000000` carry semantic meaning.
- Links use `--link: #0000cc`; secondary text uses `--muted: #404040`.

Do not add decorative colors. Color never carries state alone; labels, borders, and position must also communicate it.

### Shape and depth

- All surfaces are square.
- One-pixel and two-pixel light/dark borders create classic raised, sunken, and pressed controls.
- Do not use patterns, gradients, soft shadows, glass, glow, blur, rounded cards, or floating panels.
- The outer workbench is one application window. Internal rules and fieldsets establish ownership; do not box every datum.

### Spacing

- Dense form groups use 5–12 px gaps.
- Major regions use 16–20 px gaps.
- The desktop first viewport must expose the file action and preview without marketing whitespace.

### Signature

The narrow **Local** divider is the only transformation cue. It shows a plain arrow at rest and three restrained packets only while conversion is running. Reduced-motion mode removes packet movement without removing status.

## Deliberate 1990s references

- The solid teal desktop, navy title bar, gray window chrome, native controls, fieldsets, underlined links, visible focus, and beveled buttons are intentional.
- The interface must not add patterned backgrounds, multiple competing title bars, rainbow accents, fake visitor counters, awards, web rings, “under construction” warnings, marquees, blinking body text, cursor trails, autoplay, fake window buttons, dead navigation, or external image requests.
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
