import { EpubPreview } from "./epub-preview.js";

const MAX_BYTES = 64 * 1024 * 1024;

const form = document.querySelector("#job-form");
const fileInput = document.querySelector("#source-file");
const dropZone = document.querySelector("#drop-zone");
const fileFacts = document.querySelector("#file-facts");
const fileName = document.querySelector("#file-name");
const fileSize = document.querySelector("#file-size");
const jobNumber = document.querySelector("#job-number");
const convertButton = document.querySelector("#convert");
const cancelButton = document.querySelector("#cancel");
const download = document.querySelector("#download");
const status = document.querySelector("#status");
const progress = document.querySelector("#progress");
const format = document.querySelector("#format");
const formatNote = document.querySelector("#format-note");
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

const epubPreview = new EpubPreview(previewFrame);
let selectedFile = null;
let worker = createWorker();
let activeJob = null;
let nextJobId = 1;
let outputUrl = null;
let sourceUrl = null;
let outputFormat = null;

function createWorker() {
  const nextWorker = new Worker("/converter.worker.js", { type: "module" });
  nextWorker.addEventListener("message", handleWorkerMessage);
  nextWorker.addEventListener("error", (error) => {
    if (worker === nextWorker) finishWithError(error.message, true);
  });
  return nextWorker;
}

function resetWorker() {
  worker?.terminate();
  worker = null;
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

function clearSelectedFile() {
  selectedFile = null;
  fileFacts.hidden = true;
  convertButton.disabled = true;
  jobNumber.textContent = "No. —";
  if (sourceUrl) URL.revokeObjectURL(sourceUrl);
  sourceUrl = null;
  previewSource.disabled = true;
  previewOpen.hidden = true;
  showEmptyPreview();
}

function setFile(file) {
  if (activeJob) return;
  clearDownload();
  clearSelectedFile();
  if (!file) {
    setStatus("Waiting", "Select a PDF to prepare a local conversion.");
    return;
  }
  if (file.size > MAX_BYTES) {
    fileInput.value = "";
    setStatus("Too large", "Use the CLI for PDFs larger than 64 MiB.", "error");
    return;
  }
  const looksLikePdf = file.type === "application/pdf" || file.name.toLowerCase().endsWith(".pdf");
  if (!looksLikePdf) {
    fileInput.value = "";
    setStatus("Wrong file", "Paprika currently accepts PDF documents only.", "error");
    return;
  }
  selectedFile = file;
  sourceUrl = URL.createObjectURL(file);
  fileName.textContent = file.name;
  fileSize.textContent = formatBytes(file.size);
  fileFacts.hidden = false;
  convertButton.disabled = false;
  previewSource.disabled = false;
  const fingerprint = `${file.name}:${file.size}:${file.lastModified}`;
  let hash = 0;
  for (const character of fingerprint) hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  jobNumber.textContent = `No. ${String(hash % 10000).padStart(4, "0")}`;
  showSourcePreview();
  setStatus("Ready", "Review the source, then start the local conversion.", "ready");
}

function clearDownload() {
  if (outputUrl) URL.revokeObjectURL(outputUrl);
  outputUrl = null;
  outputFormat = null;
  download.hidden = true;
  download.removeAttribute("href");
  previewOutput.disabled = true;
  epubPreview.clear();
  previewLimit.hidden = true;
  if (sourceUrl) showSourcePreview();
}

function showEmptyPreview() {
  epubPreview.releasePageUrls();
  previewFrame.hidden = true;
  previewFrame.removeAttribute("src");
  previewFrame.setAttribute("sandbox", "allow-same-origin");
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
  // Chrome's built-in PDF viewer refuses to load in a sandboxed frame. PDF
  // bytes still come only from the user's local file or Paprika's local output.
  previewFrame.removeAttribute("src");
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
    ? `Preview is limited to ${epubPreview.pageCount} source pages. The download contains the complete EPUB.`
    : "";
}

function setJobControlsDisabled(disabled) {
  fileInput.disabled = disabled;
  dropZone.dataset.disabled = String(disabled);
  for (const control of form.querySelectorAll("select, input[type='number']")) {
    control.disabled = disabled;
  }
}

function options() {
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

function updateOutputUI() {
  const isEpub = format.value === "epub";
  rasterOptions.hidden = isEpub;
  rasterAnalysis.hidden = isEpub;
  convertButton.textContent = isEpub ? "Make EPUB" : "Make raster PDF";
  previewOutput.textContent = isEpub ? "Generated EPUB" : "Generated PDF";
  sheet.querySelector("i").textContent = isEpub ? "EPUB" : "PDF";
  formatNote.textContent = isEpub
    ? "Selectable text, reader-controlled type size, and compact output for born-digital PDFs."
    : "Experimental fallback. Pages are rendered as images, so output is larger and text is not selectable.";

  if (isEpub) {
    sheet.style.setProperty("--sheet-ratio", 0.7);
    dimensions.textContent = "Reflowable EPUB · selectable text";
  } else {
    const output = options();
    sheet.style.setProperty("--sheet-ratio", output.width / output.height);
    dimensions.textContent = `${output.width} × ${output.height} px · ${output.dpi} dpi`;
  }
}

previewSource.addEventListener("click", showSourcePreview);
previewOutput.addEventListener("click", () => showOutputPreview());
previewPrevious.addEventListener("click", () => showOutputPreview(epubPreview.chapterIndex - 1));
previewNext.addEventListener("click", () => showOutputPreview(epubPreview.chapterIndex + 1));

format.addEventListener("change", () => {
  clearDownload();
  updateOutputUI();
});

preset.addEventListener("change", () => {
  if (preset.value !== "custom") {
    const [nextWidth, nextHeight, nextDpi] = preset.value.split("x").map(Number);
    width.value = nextWidth;
    height.value = nextHeight;
    dpi.value = nextDpi;
  }
  updateOutputUI();
});

for (const field of [width, height, dpi]) {
  field.addEventListener("input", () => {
    preset.value = "custom";
    updateOutputUI();
  });
}

fileInput.addEventListener("change", () => setFile(fileInput.files?.[0] ?? null));
for (const eventName of ["dragenter", "dragover"]) {
  dropZone.addEventListener(eventName, (event) => {
    event.preventDefault();
    dropZone.dataset.dragging = "true";
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
  clearDownload();
  const selectedFormat = format.value;
  const stem = selectedFile.name.replace(/\.pdf$/i, "");
  activeJob = {
    id: nextJobId++,
    file: selectedFile,
    format: selectedFormat,
    title: stem || "Converted document",
    options: options(),
    outputName: `${stem || "document"}.paprika.${selectedFormat}`,
  };
  setJobControlsDisabled(true);
  convertButton.disabled = true;
  cancelButton.hidden = false;
  progress.hidden = false;
  setStatus("Opening", "Loading the Rust engine and checking the document…", "working");

  const job = activeJob;
  try {
    const input = await job.file.arrayBuffer();
    if (activeJob !== job) return;
    worker ??= createWorker();
    worker.postMessage(
      {
        type: "convert",
        jobId: job.id,
        input,
        format: job.format,
        title: job.title,
        options: job.options,
      },
      [input],
    );
  } catch (error) {
    if (activeJob === job) {
      finishWithError(error instanceof Error ? error.message : String(error));
    }
  }
});

cancelButton.addEventListener("click", () => {
  resetWorker();
  activeJob = null;
  progress.hidden = true;
  cancelButton.hidden = true;
  setJobControlsDisabled(false);
  convertButton.disabled = !selectedFile;
  setStatus("Canceled", "The source file is still selected. No output was saved.", "ready");
});

function handleWorkerMessage(event) {
  const message = event.data;
  if (!activeJob || message.jobId !== activeJob.id) return;
  if (message.type === "composing") {
    setStatus("Composing", "Extracting and typesetting locally…", "working");
    return;
  }
  if (message.type === "inspected") {
    setStatus(
      "Rendering",
      `${message.pages} source page${message.pages === 1 ? "" : "s"} · rendering locally…`,
      "working",
    );
    return;
  }
  if (message.type === "error") {
    finishWithError(message.message);
    return;
  }
  if (message.type === "complete") {
    const isEpub = message.format === "epub";
    const mime = isEpub ? "application/epub+zip" : "application/pdf";
    const label = isEpub ? "EPUB" : "PDF";
    const blob = new Blob([message.output], { type: mime });
    outputUrl = URL.createObjectURL(blob);
    outputFormat = message.format;
    download.href = outputUrl;
    download.download = activeJob?.outputName ?? `paprika-output.${message.format}`;
    download.textContent = `Download ${formatBytes(blob.size)} ${label}`;
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
          previewLimit.textContent = "The complete EPUB is ready, but this document exceeded the bounded browser preview.";
        }
      } else {
        showOutputPreview(0);
      }
    } catch (error) {
      previewOutput.disabled = true;
      showSourcePreview();
      previewLimit.hidden = false;
      previewLimit.textContent = `The output is ready, but its embedded preview could not be shown: ${error instanceof Error ? error.message : String(error)}`;
    }

    const pageSummary = message.pages
      ? `${message.pages} source page${message.pages === 1 ? "" : "s"}. `
      : "";
    setStatus(
      "Ready to download",
      `${pageSummary}Conversion finished locally. The source file was not uploaded.`,
      "success",
    );
    finishJob();
  }
}

function finishWithError(message, workerFailed = false) {
  setStatus("Could not convert", message || "The conversion failed.", "error");
  if (workerFailed) resetWorker();
  finishJob();
}

function finishJob() {
  activeJob = null;
  progress.hidden = true;
  cancelButton.hidden = true;
  setJobControlsDisabled(false);
  convertButton.disabled = !selectedFile;
}

window.addEventListener("pagehide", (event) => {
  if (event.persisted) return;
  worker?.terminate();
  worker = null;
  epubPreview.clear();
  if (sourceUrl) URL.revokeObjectURL(sourceUrl);
  if (outputUrl) URL.revokeObjectURL(outputUrl);
});

updateOutputUI();
