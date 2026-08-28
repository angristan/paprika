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

function updateSheet() {
  const output = options();
  const ratio = output.width / output.height;
  sheet.style.setProperty("--sheet-ratio", ratio);
  dimensions.textContent = `${output.width} × ${output.height} px · ${output.dpi} dpi`;
}

preset.addEventListener("change", () => {
  if (preset.value !== "custom") {
    const [nextWidth, nextHeight, nextDpi] = preset.value.split("x").map(Number);
    width.value = nextWidth;
    height.value = nextHeight;
    dpi.value = nextDpi;
  }
  updateSheet();
});

for (const field of [width, height, dpi]) {
  field.addEventListener("input", () => {
    preset.value = "custom";
    updateSheet();
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
  activeJob = {
    file: selectedFile,
    options: options(),
    outputName: `${selectedFile.name.replace(/\.pdf$/i, "")}.paprika.pdf`,
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
    worker.postMessage({ type: "convert", input, options: job.options }, [input]);
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
      `${message.pages} source page${message.pages === 1 ? "" : "s"} · rendering and reflowing locally…`,
      "working",
    );
    return;
  }
  if (message.type === "error") {
    finishWithError(message.message);
    return;
  }
  if (message.type === "complete") {
    const blob = new Blob([message.output], { type: "application/pdf" });
    outputUrl = URL.createObjectURL(blob);
    download.href = outputUrl;
    download.download = activeJob?.outputName ?? "paprika-output.pdf";
    download.textContent = `Download ${formatBytes(blob.size)} PDF`;
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

updateSheet();
