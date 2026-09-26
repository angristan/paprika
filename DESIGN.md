# Paprika interface

Paprika converts a local PDF into an EPUB or an experimental raster PDF. The interface should help the reader select a file, choose options, convert, inspect the result, and download it.

## Layout and copy

Use direct labels: Source PDF, Output, Preview, and Download. Avoid slogans, mock documents, device illustrations, and decorative status indicators.

On wide screens, place the introduction beside the form and preview. After file selection, shorten the introduction and give its space to the document. On mobile, show the form first and reveal the preview after file selection. A successful conversion replaces the form with the source identity and an Edit button. Place Download above the result preview.

The empty preview explains how to populate it. Processing and error states use text and the live status region. Show source identity, limits, warnings, and output metadata when they help the reader act.

## Styling

Use `Georgia, "Times New Roman", serif` for the wordmark, main heading, and empty preview. Use system fonts for the controls and supporting text: `-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`. Generated EPUB content retains its document typography inside the preview. No font downloads are required.

Use a warm paper background, brown text, and paprika red for the file picker and active primary actions. Keep controls at a 4 px corner radius and the file picker and preview at 8 px. The empty preview uses a blank page with an instruction, without simulated document content. Group fields with spacing rather than nested cards.

## Interaction

- Use native controls, visible labels, and a visible keyboard focus outline.
- Keep primary controls at least 44 px high.
- Preserve source and result preview boundaries.
- Keep warnings and errors readable without relying on color.
- Keep filenames and diagnostics within the viewport at 320 px and 200% zoom.
- Edit restores the form without discarding the output. Cancel keeps the selected source.
- Respect reduced motion for preview navigation.
- Paginate EPUB previews at a readable text size within the available height. Previous/Next traverses each screen of a source page before moving to the next source page. Recalculate pagination when the reader resizes; keep the downloaded EPUB unchanged.

## Privacy

Document bytes stay in the browser tab and Web Worker. The app must not add remote conversion, uploads, analytics, or third-party asset requests. Cloudflare serves the static app files and can observe normal HTTP metadata for those requests. See [the privacy model](docs/privacy.md).
