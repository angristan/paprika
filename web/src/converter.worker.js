import init, {
  convert_pdf_to_epub,
  inspect_pdf,
  optimize_pdf_bytes,
} from "./pkg/paprika_wasm.js";

let initialized;

async function ensureInitialized() {
  initialized ??= init().catch((error) => {
    initialized = undefined;
    throw error;
  });
  await initialized;
}

self.addEventListener("message", async (event) => {
  if (event.data.type !== "convert") return;
  const jobId = event.data.jobId;
  try {
    await ensureInitialized();
    const input = new Uint8Array(event.data.input);
    if (event.data.format === "pdf") {
      const pages = inspect_pdf(input);
      self.postMessage({ type: "inspected", jobId, pages, format: "pdf" });
      const output = optimize_pdf_bytes(input, event.data.options);
      self.postMessage(
        { type: "complete", jobId, output: output.buffer, format: "pdf", pages },
        [output.buffer],
      );
      return;
    }

    self.postMessage({ type: "composing", jobId, format: "epub" });
    const conversion = convert_pdf_to_epub(input, event.data.title);
    try {
      const preview = conversion.preview_manifest();
      const previewAssets = [];
      const transfer = [];
      for (let index = 0; index < conversion.preview_asset_count(); index += 1) {
        const asset = conversion.take_preview_asset(index);
        previewAssets.push(asset.buffer);
        transfer.push(asset.buffer);
      }
      const output = conversion.take_output();
      transfer.push(output.buffer);
      self.postMessage(
        {
          type: "complete",
          jobId,
          output: output.buffer,
          format: "epub",
          pages: conversion.source_pages,
          textPages: conversion.text_pages,
          imageCount: conversion.image_count,
          warnings: conversion.warnings(),
          preview,
          previewAssets,
        },
        transfer,
      );
    } finally {
      conversion.free();
    }
  } catch (error) {
    self.postMessage({
      type: "error",
      jobId,
      message: error instanceof Error ? error.message : String(error),
    });
  }
});
