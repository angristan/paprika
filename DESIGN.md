# Paprika interface contract

Paprika is a local document tool, not a product landing page. The interface must help a reader select one PDF, understand the limits, choose an output, run the job, inspect the result, and download it. Every visible element must support that sequence.

## Product model

- **Audience:** people making papers and documents readable on small screens.
- **Primary job:** convert one local born-digital PDF to a compact, reflowable EPUB.
- **Fallback:** offer raster PDF as an explicit experimental option for scans or layouts that semantic extraction cannot preserve.
- **Active scope:** the file selected in the current browser tab.
- **Consequential actions:** local conversion and output download. Neither changes the source file.
- **States:** empty, source ready, processing, output ready, failed, and canceled.
- **Evidence:** show file identity, output format, progress, limits, warnings, preview state, and download size. Do not invent quality scores, speed claims, or success metrics.

## Creative brief: Paprika Proof Press

- Treat the interface as a contemporary proof press, not a dashboard or nostalgic print-shop imitation.
- The solid Paprika source slab is the rigid input page. The cool proofing field is the flexible reader output. A black **REFLOW** gate connects them.
- The source slab carries the task statement because it is also the real intake and settings surface; there is no detached marketing hero.
- The empty proof shows one fixed PDF page becoming one narrow EPUB page. Its lines and labels explain the operation rather than decorate empty space.
- On success, contract the source slab into an editable file-to-format docket and expand the generated proof.

## Task hierarchy

1. Put the PDF → local composition → EPUB route in the black press header.
2. Put source selection, limits, output settings, and the primary conversion action on the Paprika source slab.
3. Keep advanced raster controls hidden until raster output is selected.
4. Keep preview, live status, report, warnings, and download together on the proofing field.
5. After conversion, retain source identity and a visible **Edit** action without competing with the proof.
6. Put the concise privacy and OCR statement directly below the press.
7. At 320 px and 200% zoom, stack route, source, reflow gate, and proof in DOM order without horizontal scrolling.

## Visual system

- **Color:** cool desk/proof colors are `#dcebed`, `#eef7f8`, and `#d2e3e6`; paper is `#ffffff`; ink is `#172023`; secondary ink and rules use `#536166`, `#667479`, `#9aa8ab`, `#dddddd`, and `#f1f1f1`. The source/action family is `#a62b1f`, `#781e16`, and `#ffe7e1`. Success is `#175b48`; error is `#8b2119` with `#fff4f1`. Do not introduce cream, beige, parchment, sepia, or warm-paper colors.
- **Type:** application chrome uses the operating-system UI stack. The source task statement uses the same family at display scale, heavy weight, and tight natural tracking. Monospace is only for real machine values. Serif text is limited to generated EPUB content.
- **Scale:** core UI uses 12–19 px. The source task statement uses 40–72 px because it is integrated into the active conversion slab, not a detached hero. At narrow widths it remains 43 px and wraps to a deliberate four-line block.
- **Shape:** the press and its planes are square. Familiar controls may use at most a 2 px radius. The 2–3 px ink rules are structural, not decorative card borders.
- **Depth:** use solid color planes, contrast, and rules. Do not use gradients, glass, glow, blur, soft shadows, or layered floating cards.
- **Spacing:** dense inside form groups, larger between process stages. Do not apply one repeated gap everywhere.
- **Motion:** only the four line fragments in the reflow gate animate during real conversion. Reduced-motion mode makes the state change immediate.
- **Signature:** Paprika red begins as the fixed source cover. The black gate visibly reflows unequal line fragments. The output page receives a Paprika spine. This transformation must remain legible without motion.

## Explicit anti-patterns

Paprika must not use:

- detached marketing hero sections, feature-card grids, fake metrics, or testimonials;
- eyebrow copy above the main task statement;
- nostalgic editorial costume or paper texture;
- decorative cards, gradients, glass, glow, or wide shadows;
- pill-shaped controls, icon containers, icon soup, or unnecessary badges;
- colored edge stripes unrelated to the source-to-book transformation;
- crushed tracking, tiny functional text, or monospace decoration;
- equal visual weight for every region;
- color without a state, action, or source/output meaning.

The large source statement, solid source slab, reflow gate, and output spine are intentional because they directly encode the document transformation.

## Interaction and accessibility

- Keep all controls keyboard operable with a visible 3 px focus outline.
- Use native controls and semantic landmarks before custom widgets.
- Keep labels visible; placeholders are not labels.
- Status changes remain in the polite live region. Errors also use text and geometry, never color alone.
- Primary controls remain at least 44 px high. Supporting text remains at least 12 px.
- Source and generated previews identify the active document and trust boundary.
- A successful result retains source identity; **Edit** restores every setting without discarding the output.
- Cancellation keeps the source selected and states what was discarded.
- The layout must not overflow at 320 CSS pixels or 200% zoom.

## Privacy contract

Document bytes stay in the browser tab and Web Worker. The deployed app has no application server, upload route, analytics, storage binding, or remote conversion service. Runtime code must not add third-party requests. Privacy copy must still distinguish document processing from ordinary requests for the static site: Cloudflare and the user's network can observe normal HTTP metadata when the app assets are loaded. See [`docs/privacy.md`](docs/privacy.md).
