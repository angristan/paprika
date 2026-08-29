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

let selectedFile = null;
let worker = null;
let activeJob = null;
let outputUrl = null;

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
  fileName.textContent = file.name;
  fileSize.textContent = formatBytes(file.size);
  fileFacts.hidden = false;
  convertButton.disabled = false;
  const fingerprint = `${file.name}:${file.size}:${file.lastModified}`;
  let hash = 0;
  for (const character of fingerprint) hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  jobNumber.textContent = `No. ${String(hash % 10000).padStart(4, "0")}`;
  setStatus("Ready", "Review the job ticket, then start the local conversion.", "ready");
}

function clearDownload() {
  if (outputUrl) URL.revokeObjectURL(outputUrl);
  outputUrl = null;
  download.hidden = true;
  download.removeAttribute("href");
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
  const outputFormat = format.value;
  const stem = selectedFile.name.replace(/\.pdf$/i, "");
  activeJob = {
    file: selectedFile,
    format: outputFormat,
    title: stem || "Converted document",
    options: options(),
    outputName: `${stem || "document"}.paprika.${outputFormat}`,
  };
  setJobControlsDisabled(true);
  convertButton.disabled = true;
  cancelButton.hidden = false;
  progress.hidden = false;
  setStatus("Opening", "Loading the Rust engine and checking the document…", "working");

  const job = activeJob;
  worker = new Worker("/converter.worker.js", { type: "module" });
  worker.addEventListener("message", handleWorkerMessage);
  worker.addEventListener("error", (error) => finishWithError(error.message));
  try {
    const input = await job.file.arrayBuffer();
    if (activeJob !== job) return;
    worker.postMessage(
      {
        type: "convert",
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
  worker?.terminate();
  worker = null;
  activeJob = null;
  progress.hidden = true;
  cancelButton.hidden = true;
  setJobControlsDisabled(false);
  convertButton.disabled = !selectedFile;
  setStatus("Canceled", "The source file is still selected. No output was saved.", "ready");
});

function handleWorkerMessage(event) {
  const message = event.data;
  if (message.type === "inspected") {
    setStatus(
      "Composing",
      message.format === "epub"
        ? `${message.pages} source page${message.pages === 1 ? "" : "s"} · extracting and typesetting locally…`
        : `${message.pages} source page${message.pages === 1 ? "" : "s"} · rendering locally…`,
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
    download.href = outputUrl;
    download.download = activeJob?.outputName ?? `paprika-output.${message.format}`;
    download.textContent = `Download ${formatBytes(blob.size)} ${label}`;
    download.hidden = false;
    setStatus("Ready to download", "Conversion finished. The source file was not uploaded.", "success");
    finishWorker();
  }
}

function finishWithError(message) {
  setStatus("Could not convert", message || "The conversion failed.", "error");
  finishWorker();
}

function finishWorker() {
  worker?.terminate();
  worker = null;
  activeJob = null;
  progress.hidden = true;
  cancelButton.hidden = true;
  setJobControlsDisabled(false);
  convertButton.disabled = !selectedFile;
}

updateOutputUI();
