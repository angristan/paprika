# Paprika contributor notes

- Keep `paprika-core` independent from filesystems, browsers, and native libraries.
- The CLI and browser must use the same `paprika-pdf` pipeline and typed options.
- Browser conversion stays local. Do not upload documents or add server-side processing.
- Keep peak memory bounded by processing one source page at a time where practical.
- This is a clean-room implementation. Do not translate or copy k2pdfopt source code.
- Run `bun run check` before submitting changes.
