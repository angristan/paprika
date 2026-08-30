import { EpubPreview } from "./epub-preview.js";

const MAX_BYTES = 64 * 1024 * 1024;
const SIMD_PROBE = Uint8Array.of(
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
  0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
  0x03, 0x02, 0x01, 0x00,
  0x0a, 0x09, 0x01, 0x07, 0x00, 0x41, 0x00, 0xfd, 0x0f, 0x1a, 0x0b,
);

const appShell = document.querySelector("#app-shell");
const form = document.querySelector("#job-form");
const fileInput = document.querySelector("#source-file");
const dropZone = document.querySelector("#drop-zone");
const dropTitle = document.querySelector("#drop-title");
const dropHelp = document.querySelector("#drop-help");
const fileFacts = document.querySelector("#file-facts");
const fileName = document.querySelector("#file-name");
const fileSize = document.querySelector("#file-size");
const convertButton = document.querySelector("#convert");
const cancelButton = document.querySelector("#cancel");
const download = document.querySelector("#download");
const status = document.querySelector("#status");
const progress = document.querySelector("#progress");
const format = document.querySelector("#format");
const formatNote = document.querySelector("#format-note");
const epubOptions = document.querySelector("#epub-options");
const bookTitle = document.querySelector("#book-title");
const bookLanguage = document.querySelector("#book-language");
const rasterOptions = document.querySelector("#raster-options");
const rasterAnalysis = document.querySelector("#raster-analysis");
const preset = document.querySelector("#preset");
const width = document.querySelector("#width");
const height = document.querySelector("#height");
const dpi = document.querySelector("#dpi");
const sheet = document.querySelector("#sheet");
const dimensions = document.querySelector("#dimensions");
const previewFrame = document.querySelector("#preview-frame");
const previewSource = document.querySelector("#preview-source");
const previewOutput = document.querySelector("#preview-output");
const previewControls = document.querySelector("#preview-controls");
const previewPrevious = document.querySelector("#preview-previous");
const previewNext = document.querySelector("#preview-next");
const previewPosition = document.querySelector("#preview-position");
const previewOpen = document.querySelector("#preview-open");
const previewLimit = document.querySelector("#preview-limit");
const resultSummary = document.querySelector("#result-summary");
const warningCount = document.querySelector("#warning-count");
const sourcePageCount = document.querySelector("#source-page-count");
const textPageCount = document.querySelector("#text-page-count");
const imageCount = document.querySelector("#image-count");
const warningGroups = document.querySelector("#warning-groups");
const diagnostics = document.querySelector("#diagnostics");
const diagnosticText = document.querySelector("#diagnostic-text");
const copyDiagnosticButton = document.querySelector("#copy-diagnostic");
const copyStatus = document.querySelector("#copy-status");
const routeSource = document.querySelector("#route-source");
const routeCompose = document.querySelector("#route-compose");
const routeResult = document.querySelector("#route-result");
const routeResultFormat = document.querySelector("#route-result-format");

const epubPreview = new EpubPreview(previewFrame);
let selectedFile = null;
let worker = null;
let workerRecycleTimer = null;
let activeJob = null;
let nextJobId = 1;
let outputUrl = null;
let sourceUrl = null;
let outputFormat = null;

class BrowserFailure extends Error {
  constructor(code, stage, userMessage, causeName = "Error") {
    super(userMessage);
    this.name = "BrowserFailure";
    this.code = code;
    this.stage = stage;
    this.causeName = causeName;
  }
}

function hasWasmSimd() {
  try {
    return typeof WebAssembly === "object" && WebAssembly.validate(SIMD_PROBE);
  } catch {
    return false;
  }
}

function createBrowserFailure(code, stage, userMessage, error) {
  const causeName = error instanceof Error ? error.name : "Error";
  return new BrowserFailure(code, stage, userMessage, causeName);
}

function ensureWorker() {
  if (workerRecycleTimer) {
    window.clearTimeout(workerRecycleTimer);
    workerRecycleTimer = null;
  }
  if (worker) return worker;
  if (!("Worker" in window)) {
    throw new BrowserFailure(
      "worker-unavailable",
      "worker-constructor",
      "This browser cannot run a local conversion worker. Use a current browser or the Paprika CLI.",
      "UnsupportedFeature",
    );
  }
  if (typeof WebAssembly !== "object") {
    throw new BrowserFailure(
      "wasm-unavailable",
      "engine-bootstrap",
      "WebAssembly is disabled or unavailable. Enable it, reload, and try again.",
      "UnsupportedFeature",
    );
  }
  if (!hasWasmSimd()) {
    throw new BrowserFailure(
      "wasm-simd-unsupported",
      "engine-bootstrap",
      "This browser does not support the WebAssembly SIMD required by Paprika. Use a current browser or the CLI.",
      "UnsupportedFeature",
    );
  }

  let nextWorker;
  try {
    nextWorker = new Worker(new URL("./converter.worker.js", import.meta.url), {
      type: "module",
      name: "paprika-converter",
    });
  } catch (error) {
    throw createBrowserFailure(
      "worker-constructor-failed",
      "worker-constructor",
      "The browser could not start the local conversion worker. Reload and try again.",
      error,
    );
  }

  worker = nextWorker;
  nextWorker.addEventListener("message", handleWorkerMessage);
  nextWorker.addEventListener("error", (event) => {
    event.preventDefault();
    if (worker !== nextWorker) return;
    resetWorker(nextWorker);
    if (!activeJob) return;
    finishWithError(
      createBrowserFailure(
        "worker-runtime-error",
        "worker-runtime",
        "The local conversion worker stopped unexpectedly. Your source is still selected; retry the conversion.",
        event.error ?? new Error(event.message || "Worker error"),
      ),
    );
  });
  nextWorker.addEventListener("messageerror", () => {
    if (worker !== nextWorker) return;
    resetWorker(nextWorker);
    if (!activeJob) return;
    finishWithError(
      new BrowserFailure(
        "worker-message-error",
        "worker-message",
        "The browser could not read the conversion result. Your source is still selected; retry the conversion.",
        "DataCloneError",
      ),
    );
  });
  return nextWorker;
}

function resetWorker(target = worker) {
  if (workerRecycleTimer) window.clearTimeout(workerRecycleTimer);
  workerRecycleTimer = null;
  target?.terminate();
  if (worker === target) worker = null;
}

function scheduleWorkerRecycle(sourceSize) {
  if (!worker) return;
  if (workerRecycleTimer) window.clearTimeout(workerRecycleTimer);
  // WebAssembly linear memory cannot shrink. Release large conversion heaps
  // immediately and smaller idle heaps after a short reuse window.
  const delay = sourceSize >= 16 * 1024 * 1024 ? 0 : 60_000;
  const target = worker;
  workerRecycleTimer = window.setTimeout(() => {
    workerRecycleTimer = null;
    if (!activeJob && worker === target) resetWorker(target);
  }, delay);
}

function formatBytes(bytes) {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function setStatus(label, message, state = "idle") {
  status.dataset.state = state;
  status.querySelector(".status-label").textContent = label;
  status.querySelector("p").textContent = message;
}

function setFlow(state) {
  const routeStates = {
    empty: ["current", "pending", "pending"],
    ready: ["done", "current", "pending"],
    working: ["done", "current", "pending"],
    success: ["done", "done", "done"],
    error: ["done", "error", "pending"],
  }[state] ?? ["current", "pending", "pending"];
  const currentStep = state === "success" ? 2 : state === "empty" ? 0 : 1;
  [routeSource, routeCompose, routeResult].forEach((step, index) => {
    step.dataset.state = routeStates[index];
    if (index === currentStep) {
      step.setAttribute("aria-current", "step");
    } else {
      step.removeAttribute("aria-current");
    }
  });

  routeSource.querySelector("span").textContent = state === "empty" ? "Source" : "Selected";
  routeCompose.querySelector("span").textContent = state === "working"
    ? "Working"
    : state === "success"
      ? "Composed"
      : state === "error"
        ? "Stopped"
        : "Compose";
  routeResult.querySelector("span").textContent = state === "success" ? "Ready" : "Result";
}

function blankPreviewFrame() {
  previewFrame.hidden = true;
  previewFrame.src = "about:blank";
  previewFrame.setAttribute("sandbox", "allow-same-origin");
  previewFrame.title = "Document preview";
}

function releaseSourceUrl() {
  if (!sourceUrl) return;
  URL.revokeObjectURL(sourceUrl);
  sourceUrl = null;
}

function clearSelectedFile() {
  blankPreviewFrame();
  epubPreview.clear();
  releaseSourceUrl();
  selectedFile = null;
  fileFacts.hidden = true;
  convertButton.disabled = true;
  previewSource.disabled = true;
  previewOpen.hidden = true;
  dropTitle.textContent = "Choose a PDF";
  dropHelp.textContent = "or drop one here";
  appShell.dataset.hasFile = "false";
  appShell.dataset.hasOutput = "false";
  showEmptyPreview();
}

function setFile(file) {
  if (activeJob) return;
  clearDownload();
  clearDiagnostics();
  clearSelectedFile();
  fileInput.value = "";
  if (!file) {
    setFlow("empty");
    setStatus("", "", "idle");
    return;
  }
  if (file.size === 0) {
    setFlow("error");
    setStatus("Empty file", "Choose a PDF that contains document data.", "error");
    return;
  }
  if (file.size > MAX_BYTES) {
    setFlow("error");
    setStatus("Too large", "Use the CLI for PDFs larger than 64 MiB.", "error");
    return;
  }
  const looksLikePdf = file.type === "application/pdf" || file.name.toLowerCase().endsWith(".pdf");
  if (!looksLikePdf) {
    setFlow("error");
    setStatus("Wrong file", "Paprika currently accepts PDF documents only.", "error");
    return;
  }

  selectedFile = file;
  sourceUrl = URL.createObjectURL(file);
  const stem = file.name.replace(/\.pdf$/i, "").trim();
  bookTitle.value = stem || "Converted document";
  bookTitle.setCustomValidity("");
  fileName.textContent = file.name;
  fileSize.textContent = formatBytes(file.size);
  fileFacts.hidden = false;
  convertButton.disabled = false;
  previewSource.disabled = false;
  dropTitle.textContent = "Replace PDF";
  dropHelp.textContent = "or drop another here";
  appShell.dataset.hasFile = "true";
  showSourcePreview();
  setFlow("ready");
  setStatus("", "", "idle");
}

function clearConversionReport() {
  resultSummary.hidden = true;
  warningCount.textContent = "";
  delete warningCount.dataset.hasWarnings;
  sourcePageCount.textContent = "—";
  textPageCount.textContent = "—";
  imageCount.textContent = "—";
  warningGroups.replaceChildren();
}

function clearDownload() {
  appShell.dataset.hasOutput = "false";
  blankPreviewFrame();
  epubPreview.clear();
  if (outputUrl) URL.revokeObjectURL(outputUrl);
  outputUrl = null;
  outputFormat = null;
  download.hidden = true;
  download.removeAttribute("href");
  previewOutput.disabled = true;
  previewLimit.hidden = true;
  clearConversionReport();
  if (sourceUrl) showSourcePreview();
  else showEmptyPreview();
}

function showEmptyPreview() {
  blankPreviewFrame();
  epubPreview.releasePageUrls();
  sheet.hidden = false;
  previewControls.hidden = true;
  previewLimit.hidden = true;
  setSelectedPreviewTab("source");
}

function setSelectedPreviewTab(tab) {
  previewSource.setAttribute("aria-pressed", String(tab === "source"));
  previewOutput.setAttribute("aria-pressed", String(tab === "output"));
}

function showPdfPreview(url, title) {
  if (!url?.startsWith("blob:")) {
    throw new Error("The PDF preview URL was not created by this browser tab.");
  }
  previewFrame.src = "about:blank";
  previewFrame.removeAttribute("sandbox");
  previewFrame.title = title;
  previewFrame.src = url;
}

function showSourcePreview() {
  if (!sourceUrl) return;
  epubPreview.releasePageUrls();
  showPdfPreview(
    sourceUrl,
    `Source PDF preview — ${selectedFile?.name ?? "selected document"}`,
  );
  previewFrame.hidden = false;
  sheet.hidden = true;
  previewControls.hidden = true;
  previewLimit.hidden = true;
  previewOpen.href = sourceUrl;
  previewOpen.textContent = "Open source PDF in a new tab";
  previewOpen.hidden = false;
  setSelectedPreviewTab("source");
}

function showOutputPreview(index = epubPreview.chapterIndex) {
  if (!outputUrl || !outputFormat) return;
  sheet.hidden = true;
  previewFrame.hidden = false;
  previewOpen.href = outputUrl;
  previewOpen.textContent = "Open generated PDF in a new tab";
  previewOpen.hidden = outputFormat !== "pdf";
  setSelectedPreviewTab("output");

  if (outputFormat === "pdf") {
    epubPreview.releasePageUrls();
    showPdfPreview(outputUrl, "Generated PDF preview");
    previewControls.hidden = true;
    previewLimit.hidden = true;
    return;
  }

  const chapter = epubPreview.show(index);
  if (!chapter) {
    showEmptyPreview();
    return;
  }
  previewControls.hidden = epubPreview.pageCount <= 1;
  previewPrevious.disabled = epubPreview.chapterIndex === 0;
  previewNext.disabled = epubPreview.chapterIndex + 1 >= epubPreview.pageCount;
  previewPosition.textContent = `Preview ${epubPreview.chapterIndex + 1} of ${epubPreview.pageCount} · source page ${chapter.source_page}`;
  previewLimit.hidden = !epubPreview.truncated;
  previewLimit.textContent = epubPreview.truncated
    ? `Showing ${epubPreview.pageCount} pages. Download is complete.`
    : "";
}

function setJobControlsDisabled(disabled) {
  fileInput.disabled = disabled;
  dropZone.dataset.disabled = String(disabled);
  for (const control of form.querySelectorAll("select, input[type='number'], input[type='text']")) {
    control.disabled = disabled;
  }
}

function rasterConversionOptions() {
  return {
    mode: document.querySelector("#mode").value,
    width: Number(width.value),
    height: Number(height.value),
    dpi: Number(dpi.value),
    sourceDpi: Number(document.querySelector("#source-dpi").value),
    margin: Number(document.querySelector("#margin").value),
    fontSize: Number(document.querySelector("#font-size").value),
    threshold: Number(document.querySelector("#threshold").value),
    columns: Number(document.querySelector("#columns").value),
  };
}

function canonicalLanguageTag() {
  const value = bookLanguage.value.trim();
  if (!value || value.length > 35) return null;
  try {
    const [canonical] = Intl.getCanonicalLocales(value);
    return canonical && canonical.length <= 35 ? canonical : null;
  } catch {
    return null;
  }
}

function validateMetadata() {
  bookTitle.setCustomValidity(
    bookTitle.value.trim() ? "" : "Enter a title for the generated EPUB.",
  );
  const language = canonicalLanguageTag();
  bookLanguage.setCustomValidity(
    language ? "" : "Enter a valid BCP 47 language tag, such as en, fr, or pt-BR.",
  );
  if (language) bookLanguage.value = language;
  return form.reportValidity() ? language : null;
}

function updateOutputUI() {
  const isEpub = format.value === "epub";
  epubOptions.hidden = !isEpub;
  rasterOptions.hidden = isEpub;
  rasterAnalysis.hidden = isEpub;
  bookTitle.disabled = !isEpub;
  bookLanguage.disabled = !isEpub;
  convertButton.textContent = isEpub ? "Make EPUB" : "Make raster PDF";
  previewOutput.textContent = isEpub ? "Result EPUB" : "Result PDF";
  routeResultFormat.textContent = isEpub ? "EPUB" : "PDF";
  formatNote.hidden = isEpub;
  formatNote.textContent = isEpub ? "" : "Experimental · image-only output.";

  dimensions.hidden = isEpub;
  if (isEpub) {
    dimensions.textContent = "";
  } else {
    const output = rasterConversionOptions();
    dimensions.textContent = `${output.width} × ${output.height} px · ${output.dpi} dpi`;
  }
}

function invalidateCompletedOutput() {
  if (!outputUrl || activeJob) return;
  clearDownload();
  setFlow(selectedFile ? "ready" : "empty");
  setStatus("", "", "idle");
}

previewSource.addEventListener("click", showSourcePreview);
previewOutput.addEventListener("click", () => showOutputPreview());
previewPrevious.addEventListener("click", () => showOutputPreview(epubPreview.chapterIndex - 1));
previewNext.addEventListener("click", () => showOutputPreview(epubPreview.chapterIndex + 1));

format.addEventListener("change", () => {
  if (outputUrl) clearDownload();
  clearDiagnostics();
  updateOutputUI();
  setFlow(selectedFile ? "ready" : "empty");
  if (selectedFile) setStatus("", "", "idle");
});

preset.addEventListener("change", () => {
  if (preset.value !== "custom") {
    const [nextWidth, nextHeight, nextDpi] = preset.value.split("x").map(Number);
    width.value = nextWidth;
    height.value = nextHeight;
    dpi.value = nextDpi;
  }
  invalidateCompletedOutput();
  updateOutputUI();
});

for (const field of [width, height, dpi]) {
  field.addEventListener("input", () => {
    preset.value = "custom";
    invalidateCompletedOutput();
    updateOutputUI();
  });
}

for (const field of [bookTitle, bookLanguage]) {
  field.addEventListener("input", () => {
    field.setCustomValidity("");
    invalidateCompletedOutput();
  });
}
bookLanguage.addEventListener("blur", () => {
  const language = canonicalLanguageTag();
  if (language) bookLanguage.value = language;
});

fileInput.addEventListener("change", () => setFile(fileInput.files?.[0] ?? null));
for (const eventName of ["dragenter", "dragover"]) {
  dropZone.addEventListener(eventName, (event) => {
    event.preventDefault();
    if (!activeJob) dropZone.dataset.dragging = "true";
  });
}
for (const eventName of ["dragleave", "drop"]) {
  dropZone.addEventListener(eventName, (event) => {
    event.preventDefault();
    delete dropZone.dataset.dragging;
  });
}
dropZone.addEventListener("drop", (event) => {
  if (!activeJob) setFile(event.dataTransfer?.files?.[0] ?? null);
});

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!selectedFile || activeJob) return;

  const selectedFormat = format.value;
  const language = selectedFormat === "epub" ? validateMetadata() : null;
  if (selectedFormat === "epub" && !language) return;

  clearDownload();
  clearDiagnostics();
  const stem = selectedFile.name.replace(/\.pdf$/i, "").trim();
  activeJob = {
    id: nextJobId++,
    file: selectedFile,
    format: selectedFormat,
    title: selectedFormat === "epub" ? bookTitle.value.trim() : stem || "Converted document",
    language: language ?? "en",
    options: rasterConversionOptions(),
    outputName: `${stem || "document"}.paprika.${selectedFormat}`,
  };
  setJobControlsDisabled(true);
  convertButton.disabled = true;
  cancelButton.hidden = false;
  progress.hidden = false;
  setFlow("working");
  setStatus("Starting", "Loading…", "working");

  const job = activeJob;
  try {
    const activeWorker = ensureWorker();
    const input = await job.file.arrayBuffer();
    if (activeJob !== job) return;
    activeWorker.postMessage(
      {
        type: "convert",
        jobId: job.id,
        input,
        format: job.format,
        title: job.title,
        language: job.language,
        options: job.options,
      },
      [input],
    );
  } catch (error) {
    if (activeJob !== job) return;
    const failure = error instanceof BrowserFailure
      ? error
      : createBrowserFailure(
        "conversion-start-failed",
        "conversion-start",
        "The browser could not start the conversion. Your source is still selected; retry.",
        error,
      );
    finishWithError(failure);
  }
});

cancelButton.addEventListener("click", () => {
  resetWorker();
  activeJob = null;
  progress.hidden = true;
  cancelButton.hidden = true;
  setJobControlsDisabled(false);
  updateOutputUI();
  convertButton.disabled = !selectedFile;
  setFlow(selectedFile ? "ready" : "empty");
  setStatus("Canceled", "No output saved.", "ready");
});

function handleWorkerMessage(event) {
  const message = event.data;
  if (!message || typeof message !== "object") {
    if (activeJob) {
      resetWorker();
      finishWithError(
        new BrowserFailure(
          "worker-protocol-error",
          "worker-message",
          "The conversion worker returned an unreadable response. Retry the conversion.",
          "ProtocolError",
        ),
      );
    }
    return;
  }
  if (
    !activeJob
    || (message.jobId !== activeJob.id && !(message.type === "error" && message.jobId == null))
  ) return;

  if (message.type === "booting") {
    setStatus("Starting", "Loading…", "working");
    return;
  }
  if (message.type === "composing") {
    setStatus(
      message.format === "pdf" ? "Rendering" : "Composing",
      message.format === "pdf" ? "Rendering pages…" : "Rebuilding reading order…",
      "working",
    );
    return;
  }
  if (message.type === "inspected") {
    setStatus(
      "Rendering",
      `${message.pages} page${message.pages === 1 ? "" : "s"}…`,
      "working",
    );
    return;
  }
  if (message.type === "error") {
    resetWorker();
    finishWithError(
      new BrowserFailure(
        typeof message.code === "string" ? message.code : "conversion-failed",
        typeof message.stage === "string" ? message.stage : "conversion",
        typeof message.message === "string" && message.message
          ? message.message
          : "The conversion failed. Your source is still selected; retry.",
        typeof message.errorName === "string" ? message.errorName : "Error",
      ),
    );
    return;
  }
  if (message.type !== "complete") return;

  try {
    completeJob(message);
  } catch (error) {
    resetWorker();
    finishWithError(
      createBrowserFailure(
        "result-processing-failed",
        "result-processing",
        "The conversion finished, but the browser could not prepare the result. Retry the conversion.",
        error,
      ),
    );
  }
}

function completeJob(message) {
  if (!(message.output instanceof ArrayBuffer)) {
    throw new TypeError("The worker result did not contain a transferable output buffer.");
  }
  const completedJob = activeJob;
  const isEpub = message.format === "epub";
  const mime = isEpub ? "application/epub+zip" : "application/pdf";
  const label = isEpub ? "EPUB" : "PDF";
  const blob = new Blob([message.output], { type: mime });
  outputUrl = URL.createObjectURL(blob);
  outputFormat = message.format;
  download.href = outputUrl;
  download.download = completedJob?.outputName ?? `paprika-output.${message.format}`;
  download.textContent = `Download ${label} · ${formatBytes(blob.size)}`;
  download.hidden = false;
  previewOutput.disabled = false;

  try {
    if (isEpub) {
      if (message.preview?.chapters?.length) {
        epubPreview.setData(message.preview, message.previewAssets ?? []);
        showOutputPreview(0);
      } else {
        previewOutput.disabled = true;
        showSourcePreview();
        previewLimit.hidden = false;
        previewLimit.textContent = "Preview unavailable. Download is ready.";
      }
    } else {
      showOutputPreview(0);
    }
  } catch {
    epubPreview.clear();
    previewOutput.disabled = true;
    showSourcePreview();
    previewLimit.hidden = false;
    previewLimit.textContent = "Preview unavailable. Download is ready.";
  }

  renderConversionReport({
    format: message.format,
    pages: message.pages,
    textPages: message.textPages,
    images: message.imageCount,
    warnings: message.warnings,
  });
  appShell.dataset.hasOutput = "true";
  setFlow("success");
  setStatus("Ready", "", "success");
  finishJob();
  scheduleWorkerRecycle(completedJob?.file.size ?? 0);
}

function renderConversionReport(report) {
  const warnings = Array.isArray(report.warnings)
    ? report.warnings.filter((item) => typeof item === "string" && item.trim()).map((item) => item.trim())
    : [];
  sourcePageCount.textContent = countText(report.pages);
  textPageCount.textContent = report.format === "epub" ? countText(report.textPages) : "N/A";
  imageCount.textContent = report.format === "epub" ? countText(report.images) : "N/A";
  warningCount.textContent = warnings.length === 0
    ? "No warnings"
    : `${warnings.length} warning${warnings.length === 1 ? "" : "s"}`;
  warningCount.dataset.hasWarnings = String(warnings.length > 0);
  warningGroups.replaceChildren();

  for (const [group, items] of groupWarnings(warnings)) {
    const section = document.createElement("section");
    section.className = "warning-group";
    const heading = document.createElement("h4");
    heading.textContent = group;
    const list = document.createElement("ul");
    for (const warning of items) {
      const item = document.createElement("li");
      item.textContent = warning.length > 800 ? `${warning.slice(0, 797)}…` : warning;
      list.append(item);
    }
    section.append(heading, list);
    warningGroups.append(section);
  }
  resultSummary.hidden = false;
}

function countText(value) {
  return Number.isSafeInteger(value) && value >= 0 ? String(value) : "—";
}

function groupWarnings(warnings) {
  const groups = new Map();
  for (const warning of warnings) {
    let group = "Other conversion notes";
    if (/\b(ocr|text layer|selectable text|text extraction)\b/i.test(warning)) {
      group = "Text recovery";
    } else if (/\b(limit|budget|exceed|too large|memory|skipped)\b/i.test(warning)) {
      group = "Browser limits";
    } else if (/\b(math|equations?|figures?|images?|visual|columns?)\b/i.test(warning)) {
      group = "Visual content";
    } else if (/\b(could not|failed|decode|encode|inspect)\b/i.test(warning)) {
      group = "Processing";
    }
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group).push(warning);
  }
  return groups;
}

function finishWithError(error) {
  const failure = error instanceof BrowserFailure
    ? error
    : createBrowserFailure(
      "conversion-failed",
      "conversion",
      "The conversion failed. Your source is still selected; retry.",
      error,
    );
  setFlow("error");
  setStatus("Could not convert", failure.message, "error");
  showDiagnostics(failure);
  finishJob();
}

function finishJob() {
  activeJob = null;
  progress.hidden = true;
  cancelButton.hidden = true;
  setJobControlsDisabled(false);
  updateOutputUI();
  convertButton.disabled = !selectedFile;
}

function showDiagnostics(failure) {
  const safeCode = safeDiagnosticToken(failure.code, "unknown-error");
  const safeStage = safeDiagnosticToken(failure.stage, "unknown-stage");
  const safeErrorName = safeDiagnosticToken(failure.causeName, "Error");
  diagnosticText.textContent = [
    "Paprika browser diagnostic",
    `Code: ${safeCode}`,
    `Stage: ${safeStage}`,
    `Error type: ${safeErrorName}`,
    `Output: ${format.value === "pdf" ? "raster-pdf" : "epub"}`,
    `Worker API: ${"Worker" in window ? "available" : "unavailable"}`,
    `WebAssembly: ${typeof WebAssembly === "object" ? "available" : "unavailable"}`,
    `WebAssembly SIMD: ${hasWasmSimd() ? "available" : "unavailable"}`,
    "Document names and contents: omitted",
  ].join("\n");
  copyStatus.textContent = "";
  diagnostics.hidden = false;
  diagnostics.open = true;
}

function safeDiagnosticToken(value, fallback) {
  const token = String(value ?? "").replace(/[^a-zA-Z0-9_.-]/g, "").slice(0, 64);
  return token || fallback;
}

function clearDiagnostics() {
  diagnostics.hidden = true;
  diagnostics.open = false;
  diagnosticText.textContent = "";
  copyStatus.textContent = "";
}

copyDiagnosticButton.addEventListener("click", async () => {
  const text = diagnosticText.textContent;
  if (!text) return;
  try {
    if (!navigator.clipboard?.writeText) throw new Error("Clipboard API unavailable");
    await navigator.clipboard.writeText(text);
    copyStatus.textContent = "Copied.";
  } catch {
    const field = document.createElement("textarea");
    field.value = text;
    field.readOnly = true;
    field.setAttribute("aria-hidden", "true");
    field.style.position = "fixed";
    field.style.opacity = "0";
    document.body.append(field);
    field.select();
    const copied = document.execCommand("copy");
    field.remove();
    copyStatus.textContent = copied ? "Copied." : "Copy failed. Select the report manually.";
  }
});

window.addEventListener("pagehide", (event) => {
  if (event.persisted) return;
  activeJob = null;
  resetWorker();
  blankPreviewFrame();
  epubPreview.clear();
  releaseSourceUrl();
  if (outputUrl) URL.revokeObjectURL(outputUrl);
  outputUrl = null;
});

updateOutputUI();
setFlow("empty");
