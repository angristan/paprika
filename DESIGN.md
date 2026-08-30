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

## Creative brief: Paprika ’97 Hypertext Utility

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
Feel · exuberant, hand-built, dependable
Avoid · modern SaaS polish, illegible nostalgia, fake browser controls
```

Paprika should feel like a great shareware utility discovered on the 1997 web: bright, direct, slightly eccentric, and serious about doing one job well. This is not a generic GeoCities costume. The nostalgia must explain the product: a desktop-style local utility sits inside a hypertext page, and visible packets cross a **LOCAL LINK** from fixed PDF to reflowable book.

## Task hierarchy

1. Use the bright site masthead to identify Paprika as a local PDF → EPUB utility.
2. Keep the three real stages—PDF, local processing, output—in a compact status strip.
3. Put source selection, limits, output settings, and the primary conversion action in the left utility pane.
4. Keep advanced raster controls hidden until raster output is selected.
5. Keep preview, live status, report, warnings, and download together in the right utility pane.
6. After conversion, retain source identity and a visible **Edit** action without competing with the result.
7. Put the concise privacy and OCR statement in the page footer.
8. At 320 px and 200% zoom, stack route, source, local link, and preview in DOM order without horizontal scrolling.

## Visual system

### Type

- UI and controls: `Verdana, Tahoma, Arial, sans-serif`, regular and bold.
- Display copy and the wordmark: `Arial Black, Arial, sans-serif`, bold only.
- Machine values and the local-link label: `Courier New, Courier, monospace`, regular and bold.
- Generated EPUB content keeps its own document typography inside the sandboxed preview.
- Diagram-only labels use 8–10 px. Functional UI uses 12, 13, 14, and 15 px. Structural headings use 18, 22, 24, 28, and 30 px. Display copy uses a documented fluid 34–58 px range. No functional text is below 12 px.

### Color

The palette uses high-contrast, web-safe colors associated with late-1990s personal pages and desktop utilities.

- `--desktop: #000066` and `--desktop-dot: #66ccff` for the single tiled page background.
- `--chrome: #c0c0c0`, `--chrome-light: #ffffff`, `--chrome-mid: #808080`, and `--chrome-dark: #000000` for utility surfaces and bevels.
- `--navy: #000080` and `--blue: #0000cc` for title bars, selected state, and trusted local structure.
- `--yellow: #ffff00`, `--aqua: #00ffff`, and `--hot-pink: #ff00aa` for identity and playful, non-critical emphasis.
- `--paprika: #cc3300` and `--paprika-dark: #991f00` for the source document and primary action.
- `--success: #006600`, `--error: #990000`, `--paper: #ffffff`, and `--ink: #000000` for semantic states and reading surfaces.
- Supporting utility tokens are `#00cc00` for the local-mode lamp, `#a0a0a0`/`#b0b0b0` for disabled chrome, `#333333` for disabled ink, `#ffffcc` for control hover, `#ff9966` for the raised Paprika edge, and `#008080` for the proof field. Pattern and hard-shadow alpha values are tokens too; they are not one-off component colors.

Color never carries state alone. Every state also has text, position, and border treatment.

### Shape and depth

- All surfaces are square. Radius is always 0.
- One-pixel and two-pixel light/dark borders create classic raised, sunken, and pressed controls.
- Do not use soft shadows, glass, glow, blur, rounded cards, or floating panels.
- The outer workbench is one application window. Inside it, fieldsets and title bars establish ownership; do not wrap every datum in a box.

### Spacing

- Dense form groups use 6–12 px gaps.
- Major utility regions use 16–24 px gaps.
- The desktop first viewport must expose the file action and preview diagram without giant marketing whitespace.

### Signature

The **LOCAL LINK** is the only animated signature. Four unequal packets visibly move from the fixed-page source toward the reflowable output only while conversion is running. In all other states it remains a legible static connection. Reduced-motion mode removes movement without removing status.

## Deliberate 1990s references

- Native selects, inputs, fieldsets, underlined links, visible focus, beveled controls, saturated title bars, and a single tiled desktop background are intentional.
- The page may say “Paprika ’97” and “Best viewed with a PDF” because these are identity and task copy, not claims about compatibility.
- The interface must not add fake visitor counters, fake awards, web rings, “under construction” warnings, marquees, blinking body text, cursor trails, autoplay, fake window buttons, dead navigation, or external image requests.
- Nostalgia never overrides clear labels, contrast, touch size, error recovery, or the actual conversion flow.

## Interaction and accessibility

- Keep all controls keyboard operable with a visible 3 px yellow or navy focus outline.
- Use native controls and semantic landmarks before custom widgets.
- Keep labels visible; placeholders are not labels.
- Status changes remain in the polite live region. Errors use text and geometry, never color alone.
- Primary controls remain at least 44 px high. Supporting text remains at least 12 px.
- Source and generated previews identify the active document and trust boundary.
- A successful result retains source identity; **Edit** restores every setting without discarding the output.
- Cancellation keeps the source selected and states what was discarded.
- The layout must not overflow at 320 CSS pixels or 200% zoom.
- Touch and keyboard users receive every action that hover users receive.

## Privacy contract

Document bytes stay in the browser tab and Web Worker. The deployed app has no application server, upload route, analytics, storage binding, or remote conversion service. Runtime code must not add third-party requests. Privacy copy must still distinguish document processing from ordinary requests for the static site: Cloudflare and the user’s network can observe normal HTTP metadata when the app assets are loaded. See [`docs/privacy.md`](docs/privacy.md).
