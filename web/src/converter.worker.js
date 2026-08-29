import init, {
  convert_pdf_to_epub_bytes,
  inspect_pdf,
  optimize_pdf_bytes,
} from "./pkg/paprika_wasm.js";

let initialized;

self.addEventListener("message", async (event) => {
  if (event.data.type !== "convert") return;
  try {
    initialized ??= init();
    await initialized;
    const input = new Uint8Array(event.data.input);
    const pages = inspect_pdf(input);
    self.postMessage({ type: "inspected", pages, format: event.data.format });
    const output = event.data.format === "pdf"
      ? optimize_pdf_bytes(input, event.data.options)
      : convert_pdf_to_epub_bytes(input, event.data.title);
    self.postMessage(
      { type: "complete", output: output.buffer, format: event.data.format },
      [output.buffer],
    );
  } catch (error) {
    self.postMessage({
      type: "error",
      message: error instanceof Error ? error.message : String(error),
    });
  }
});
