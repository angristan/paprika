import init, { inspect_pdf, optimize_pdf_bytes } from "./pkg/paprika_wasm.js";

let initialized;

self.addEventListener("message", async (event) => {
  if (event.data.type !== "convert") return;
  try {
    initialized ??= init();
    await initialized;
    const input = new Uint8Array(event.data.input);
    const pages = inspect_pdf(input);
    self.postMessage({ type: "inspected", pages });
    const output = optimize_pdf_bytes(input, event.data.options);
    self.postMessage({ type: "complete", output: output.buffer }, [output.buffer]);
  } catch (error) {
    self.postMessage({
      type: "error",
      message: error instanceof Error ? error.message : String(error),
    });
  }
});
