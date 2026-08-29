# Paprika interface brief

- Audience · people reading papers and documents on small screens
- Job · turn a local born-digital PDF into a compact, reflowable EPUB
- Primary action · choose a PDF, preview it, convert locally, inspect the result, download EPUB
- Fallback · explicitly experimental raster PDF for scans or difficult layouts
- Real content · file identity, page count, output type, processing state, limitations
- Constraints · private and local-only, CPU-heavy WASM, keyboard/touch use, 320 px width

- Objects · one source document, one output format, one output document, one local preview surface
- Active scope · the file selected in this browser tab
- Critical states · empty, source preview, processing, output preview, failure, canceled
- Consequential actions · local conversion and output download; neither changes the source

- Feel: print-workshop, direct, trustworthy
- Avoid: generic SaaS dashboard, ornamental gradients, fake quality or performance claims

## Visual contract

- Type: system sans for UI and Georgia for the short editorial introduction; system monospace for status data.
- Color: warm paper background, ink foreground, paprika red only for the primary action and focus.
- Layout: a compact two-column workbench that collapses to one column; controls resemble a typesetter's job ticket.
- Shape: square paper surfaces with restrained 2–6 px radii; no floating glass cards.
- Signature: a live paper silhouette identifies reflowable EPUB or the selected raster page geometry.
- Omit: marketing sections, feature-card grids, and decorative animation.
