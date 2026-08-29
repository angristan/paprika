const WASM_MODULE_URL = "./pkg/paprika_wasm.js";
const SIMD_PROBE = Uint8Array.of(
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
  0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
  0x03, 0x02, 0x01, 0x00,
  0x0a, 0x09, 0x01, 0x07, 0x00, 0x41, 0x00, 0xfd, 0x0f, 0x1a, 0x0b,
);

let wasmModulePromise;
let initializationPromise;
let activeJobId = null;

class WorkerFailure extends Error {
  constructor(code, stage, message, cause) {
    super(message);
    this.name = "WorkerFailure";
    this.code = code;
    this.stage = stage;
    this.causeName = cause instanceof Error ? cause.name : "Error";
  }
}

function supportsWasmSimd() {
  try {
    return typeof WebAssembly === "object" && WebAssembly.validate(SIMD_PROBE);
  } catch {
    return false;
  }
}

async function ensureInitialized() {
  if (typeof WebAssembly !== "object") {
    throw new WorkerFailure(
      "wasm-unavailable",
      "engine-bootstrap",
      "WebAssembly is disabled or unavailable. Enable it, reload, and try again.",
    );
  }
  if (!supportsWasmSimd()) {
    throw new WorkerFailure(
      "wasm-simd-unsupported",
      "engine-bootstrap",
      "This browser does not support the WebAssembly SIMD required by Paprika. Use a current browser or the CLI.",
    );
  }

  try {
    wasmModulePromise ??= import(WASM_MODULE_URL);
    const wasm = await wasmModulePromise;
    initializationPromise ??= wasm.default().catch((error) => {
      initializationPromise = undefined;
      throw error;
    });
    await initializationPromise;
    return wasm;
  } catch (error) {
    wasmModulePromise = undefined;
    initializationPromise = undefined;
    const isCompileFailure =
      (typeof WebAssembly.CompileError === "function" && error instanceof WebAssembly.CompileError)
      || /simd|failed to compile|invalid opcode|illegal opcode/i.test(
        error instanceof Error ? error.message : String(error),
      );
    if (isCompileFailure) {
      throw new WorkerFailure(
        "wasm-simd-bootstrap-failed",
        "engine-bootstrap",
        "The browser could not compile Paprika's WebAssembly SIMD engine. Update the browser or use the CLI.",
        error,
      );
    }
    throw new WorkerFailure(
      "wasm-bootstrap-failed",
      "engine-bootstrap",
      "The local conversion engine could not load. Reload the page and retry.",
      error,
    );
  }
}

function transferableBuffer(bytes, label) {
  if (!(bytes instanceof Uint8Array)) {
    throw new TypeError(`${label} was not returned as bytes.`);
  }
  if (bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength) {
    return bytes.buffer;
  }
  return bytes.slice().buffer;
}

function conversionMessage(error) {
  const message = error instanceof Error ? error.message : String(error ?? "");
  if (!message || message === "undefined" || message === "null") {
    return "The document could not be converted. The source remains selected; retry or use the CLI.";
  }
  return message.length > 500 ? `${message.slice(0, 497)}…` : message;
}

function postFailure(jobId, error) {
  const failure = error instanceof WorkerFailure
    ? error
    : new WorkerFailure(
      "conversion-failed",
      "conversion",
      conversionMessage(error),
      error,
    );
  self.postMessage({
    type: "error",
    jobId,
    code: failure.code,
    stage: failure.stage,
    message: failure.message,
    errorName: failure.causeName,
  });
}

self.addEventListener("message", async (event) => {
  if (!event.data || event.data.type !== "convert") return;
  const jobId = event.data.jobId;
  activeJobId = jobId;
  try {
    if (!(event.data.input instanceof ArrayBuffer)) {
      throw new WorkerFailure(
        "worker-input-invalid",
        "worker-message",
        "The browser could not pass the source PDF to the conversion worker. Retry the conversion.",
        new TypeError("Expected an ArrayBuffer"),
      );
    }
    self.postMessage({ type: "booting", jobId });
    const wasm = await ensureInitialized();
    const input = new Uint8Array(event.data.input);

    if (event.data.format === "pdf") {
      self.postMessage({ type: "composing", jobId, format: "pdf" });
      const conversion = wasm.optimize_pdf_bytes(input, event.data.options);
      try {
        const output = conversion.take_output();
        const outputBuffer = transferableBuffer(output, "Raster PDF output");
        self.postMessage(
          {
            type: "complete",
            jobId,
            output: outputBuffer,
            format: "pdf",
            pages: conversion.source_pages,
            outputPages: conversion.output_pages,
          },
          [outputBuffer],
        );
      } finally {
        conversion.free();
      }
      return;
    }

    if (event.data.format !== "epub") {
      throw new WorkerFailure(
        "output-format-invalid",
        "conversion",
        "The selected output format is not supported.",
        new TypeError("Unknown output format"),
      );
    }

    self.postMessage({ type: "composing", jobId, format: "epub" });
    const conversion = wasm.convert_pdf_to_epub(
      input,
      event.data.title,
      event.data.language,
    );
    try {
      const preview = conversion.preview_manifest();
      const previewAssets = [];
      const transfer = [];
      for (let index = 0; index < conversion.preview_asset_count(); index += 1) {
        const asset = conversion.take_preview_asset(index);
        const assetBuffer = transferableBuffer(asset, "EPUB preview asset");
        previewAssets.push(assetBuffer);
        transfer.push(assetBuffer);
      }
      const output = conversion.take_output();
      const outputBuffer = transferableBuffer(output, "EPUB output");
      transfer.push(outputBuffer);
      self.postMessage(
        {
          type: "complete",
          jobId,
          output: outputBuffer,
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
    postFailure(jobId, error);
  } finally {
    if (activeJobId === jobId) activeJobId = null;
  }
});

self.addEventListener("messageerror", () => {
  postFailure(
    activeJobId,
    new WorkerFailure(
      "worker-input-message-error",
      "worker-message",
      "The conversion worker could not read the source data. Retry the conversion.",
      new DOMException("Worker message could not be deserialized", "DataCloneError"),
    ),
  );
});
