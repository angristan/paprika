const PREVIEW_ORIGIN = "https://preview.invalid/OEBPS/";
const ALLOWED_IMAGE_TYPES = new Set(["image/gif", "image/jpeg", "image/png", "image/webp"]);
const MAX_STYLESHEET_BYTES = 512 * 1024;
const MAX_CHAPTER_BYTES = 2 * 1024 * 1024;

export class EpubPreview {
  constructor(frame) {
    if (!(frame instanceof HTMLIFrameElement)) {
      throw new TypeError("EPUB preview requires an iframe.");
    }
    this.frame = frame;
    this.manifest = null;
    this.assetBuffers = [];
    this.chapterIndex = 0;
    this.stylesheetUrl = null;
    this.documentUrl = null;
    this.pageAssetUrls = [];
    this.pageLoadHandler = null;
    this.pageLoadGeneration = 0;
  }

  get pageCount() {
    return this.manifest?.chapters.length ?? 0;
  }

  get truncated() {
    return this.manifest?.truncated ?? false;
  }

  setData(manifest, assetBuffers) {
    validateManifest(manifest, assetBuffers);
    this.clear();
    this.manifest = manifest;
    this.assetBuffers = assetBuffers;
    this.stylesheetUrl = URL.createObjectURL(
      new Blob([manifest.stylesheet], { type: "text/css" }),
    );
  }

  show(index, onLoad) {
    if (!this.manifest || !Number.isInteger(index) || index < 0 || index >= this.pageCount) {
      return null;
    }
    this.releasePageUrls();
    this.chapterIndex = index;
    const chapter = this.manifest.chapters[index];
    const parser = new DOMParser();
    const document = parser.parseFromString(chapter.xhtml, "application/xhtml+xml");
    if (document.querySelector("parsererror") || document.documentElement.localName !== "html") {
      throw new Error("The generated EPUB preview chapter is not valid XHTML.");
    }

    document
      .querySelectorAll(
        "script, iframe, object, embed, base, style, link, form, input, button, textarea, select, option, video, audio, source, track, canvas, meta[http-equiv]",
      )
      .forEach((node) => node.remove());
    for (const element of document.querySelectorAll("*")) {
      for (const attribute of [...element.attributes]) {
        const name = attribute.name.toLowerCase();
        if (
          name.startsWith("on")
          || name === "style"
          || name === "srcdoc"
          || name === "formaction"
          || name === "action"
          || name === "poster"
          || name === "xlink:href"
        ) {
          element.removeAttribute(attribute.name);
        }
      }
    }
    document.querySelectorAll("a").forEach((link) => {
      for (const name of ["href", "target", "download", "ping", "rel"]) {
        link.removeAttribute(name);
      }
    });

    const assets = new Map(this.manifest.assets.map((asset) => [asset.href, asset]));
    document.querySelectorAll("img").forEach((image) => {
      const href = resolveEpubPath(chapter.href, image.getAttribute("src") ?? "");
      const asset = assets.get(href);
      const buffer = asset ? this.assetBuffers[asset.index] : null;
      if (!asset || !(buffer instanceof ArrayBuffer) || !ALLOWED_IMAGE_TYPES.has(asset.media_type)) {
        image.removeAttribute("src");
        image.removeAttribute("srcset");
        return;
      }
      const url = URL.createObjectURL(new Blob([buffer], { type: asset.media_type }));
      this.pageAssetUrls.push(url);
      image.setAttribute("src", url);
      image.removeAttribute("srcset");
    });

    const head = document.querySelector("head");
    if (!head) throw new Error("The generated EPUB preview has no document head.");
    const csp = document.createElementNS("http://www.w3.org/1999/xhtml", "meta");
    csp.setAttribute("http-equiv", "Content-Security-Policy");
    csp.setAttribute(
      "content",
      "default-src 'none'; img-src blob:; style-src blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'",
    );
    head.prepend(csp);
    const pageLoadGeneration = ++this.pageLoadGeneration;
    const pageLoadId = String(pageLoadGeneration);
    const pageMarker = document.createElementNS("http://www.w3.org/1999/xhtml", "meta");
    pageMarker.setAttribute("name", "paprika-preview-page");
    pageMarker.setAttribute("content", pageLoadId);
    head.append(pageMarker);
    const stylesheet = document.createElementNS("http://www.w3.org/1999/xhtml", "link");
    stylesheet.setAttribute("rel", "stylesheet");
    stylesheet.setAttribute("href", this.stylesheetUrl);
    head.append(stylesheet);

    const serialized = new XMLSerializer().serializeToString(document);
    this.documentUrl = URL.createObjectURL(
      new Blob([serialized], { type: "application/xhtml+xml" }),
    );
    const documentUrl = this.documentUrl;
    let waitingForBlankReset = this.frame.contentDocument?.URL !== "about:blank";
    this.pageLoadHandler = () => {
      if (this.pageLoadGeneration !== pageLoadGeneration) return;
      const loadedDocument = this.frame.contentDocument;
      const loadedPageId = loadedDocument
        ?.querySelector('meta[name="paprika-preview-page"]')
        ?.getAttribute("content");
      if (loadedPageId === pageLoadId) {
        this.cancelPendingPageLoad();
        if (typeof onLoad === "function") onLoad();
        return;
      }
      if (waitingForBlankReset && loadedDocument?.URL === "about:blank") {
        waitingForBlankReset = false;
        this.frame.src = documentUrl;
      }
    };
    this.frame.addEventListener("load", this.pageLoadHandler);

    this.frame.title = `Generated EPUB preview — ${chapter.title}, source page ${chapter.source_page}`;
    this.frame.setAttribute("sandbox", "allow-same-origin");
    if (waitingForBlankReset) {
      // WebKit can commit this reset after an immediately assigned blob URL. Wait for
      // the blank document before starting the final chapter navigation.
      this.frame.src = "about:blank";
    } else {
      this.frame.src = documentUrl;
    }
    return chapter;
  }

  clear() {
    this.releasePageUrls();
    if (this.stylesheetUrl) URL.revokeObjectURL(this.stylesheetUrl);
    this.stylesheetUrl = null;
    this.manifest = null;
    this.assetBuffers = [];
    this.chapterIndex = 0;
  }

  cancelPendingPageLoad() {
    if (!this.pageLoadHandler) return;
    this.frame.removeEventListener("load", this.pageLoadHandler);
    this.pageLoadHandler = null;
  }

  releasePageUrls() {
    this.cancelPendingPageLoad();
    if (this.documentUrl) URL.revokeObjectURL(this.documentUrl);
    for (const url of this.pageAssetUrls) URL.revokeObjectURL(url);
    this.documentUrl = null;
    this.pageAssetUrls = [];
  }
}

function validateManifest(manifest, assetBuffers) {
  if (!manifest || typeof manifest !== "object") {
    throw new TypeError("The EPUB preview manifest is missing.");
  }
  if (typeof manifest.stylesheet !== "string" || manifest.stylesheet.length > MAX_STYLESHEET_BYTES) {
    throw new TypeError("The EPUB preview stylesheet is invalid.");
  }
  if (!Array.isArray(manifest.chapters) || !Array.isArray(manifest.assets)) {
    throw new TypeError("The EPUB preview manifest is incomplete.");
  }
  if (!Array.isArray(assetBuffers)) {
    throw new TypeError("The EPUB preview assets are missing.");
  }

  for (const chapter of manifest.chapters) {
    if (
      !chapter
      || typeof chapter.href !== "string"
      || typeof chapter.title !== "string"
      || typeof chapter.xhtml !== "string"
      || chapter.xhtml.length > MAX_CHAPTER_BYTES
      || !Number.isSafeInteger(chapter.source_page)
      || chapter.source_page < 1
      || !resolveEpubPath("text/placeholder.xhtml", chapter.href)
    ) {
      throw new TypeError("An EPUB preview chapter is invalid.");
    }
  }

  for (const asset of manifest.assets) {
    if (
      !asset
      || typeof asset.href !== "string"
      || !Number.isSafeInteger(asset.index)
      || asset.index < 0
      || asset.index >= assetBuffers.length
      || typeof asset.media_type !== "string"
      || !resolveEpubPath("text/placeholder.xhtml", asset.href)
      || !(assetBuffers[asset.index] instanceof ArrayBuffer)
    ) {
      throw new TypeError("An EPUB preview asset is invalid.");
    }
  }
}

function resolveEpubPath(chapterHref, resourceHref) {
  try {
    const chapter = new URL(chapterHref, PREVIEW_ORIGIN);
    const resource = new URL(resourceHref, chapter);
    if (resource.origin !== new URL(PREVIEW_ORIGIN).origin) return "";
    const path = resource.pathname.replace(/^\/OEBPS\//, "");
    return path && !path.startsWith("/") ? decodeURIComponent(path) : "";
  } catch {
    return "";
  }
}
