# Paprika interface brief

- Audience · people reading research papers and scanned documents on small screens
- Job · turn a local PDF into a more readable device-sized PDF
- Primary action · choose a PDF, tune a small set of output controls, convert, download
- Real content · file identity, page count, output geometry, processing state, limitations
- Constraints · private and local-only, CPU-heavy WASM, keyboard/touch use, 320 px width

- Objects · one source document, one conversion configuration, one output document
- Active scope · the file selected in this browser tab
- Critical states · empty, ready, processing, success, failure, canceled
- Consequential actions · local conversion and output download; neither changes the source

- Feel: print-workshop, direct, trustworthy
- Avoid: generic SaaS dashboard, ornamental gradients, fake performance claims

## Visual contract

- Type: system sans for UI and Georgia for the short editorial introduction; system monospace for dimensions and status data.
- Color: warm paper background, ink foreground, paprika red only for the primary action and focus.
- Layout: a compact two-column workbench that collapses to one column; controls resemble a typesetter's job ticket.
- Shape: square paper surfaces with restrained 2–6 px radii; no floating glass cards.
- Signature: output dimensions are shown as a live, proportional sheet silhouette.
- Omit: marketing sections, feature-card grids, and decorative animation.
