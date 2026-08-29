const PREVIEW_ORIGIN = "https://preview.invalid/OEBPS/";

export class EpubPreview {
  constructor(frame) {
    this.frame = frame;
    this.manifest = null;
    this.assetBuffers = [];
    this.chapterIndex = 0;
    this.stylesheetUrl = null;
    this.documentUrl = null;
    this.pageAssetUrls = [];
  }

  get pageCount() {
    return this.manifest?.chapters.length ?? 0;
  }

  get truncated() {
    return this.manifest?.truncated ?? false;
  }

  setData(manifest, assetBuffers) {
    this.clear();
    this.manifest = manifest;
    this.assetBuffers = assetBuffers;
    this.stylesheetUrl = URL.createObjectURL(
      new Blob([manifest.stylesheet], { type: "text/css" }),
    );
    return this.show(0);
  }

  show(index) {
    if (!this.manifest || index < 0 || index >= this.pageCount) return null;
    this.releasePageUrls();
    this.chapterIndex = index;
    const chapter = this.manifest.chapters[index];
    const parser = new DOMParser();
    const document = parser.parseFromString(chapter.xhtml, "application/xhtml+xml");
    if (document.querySelector("parsererror")) {
      throw new Error("The generated EPUB preview chapter is not valid XHTML.");
    }

    document.querySelectorAll("script, iframe, object, embed, base, style, link").forEach((node) => node.remove());
    for (const element of document.querySelectorAll("*")) {
      for (const attribute of [...element.attributes]) {
        if (attribute.name.toLowerCase().startsWith("on")) element.removeAttribute(attribute.name);
      }
    }
    document.querySelectorAll("a").forEach((link) => {
      link.removeAttribute("href");
      link.removeAttribute("target");
    });

    const assets = new Map(this.manifest.assets.map((asset) => [asset.href, asset]));
    document.querySelectorAll("img").forEach((image) => {
      const href = resolveEpubPath(chapter.href, image.getAttribute("src") ?? "");
      const asset = assets.get(href);
      const buffer = asset ? this.assetBuffers[asset.index] : null;
      if (!asset || !buffer) {
        image.removeAttribute("src");
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
    const stylesheet = document.createElementNS("http://www.w3.org/1999/xhtml", "link");
    stylesheet.setAttribute("rel", "stylesheet");
    stylesheet.setAttribute("href", this.stylesheetUrl);
    head.append(stylesheet);

    const serialized = new XMLSerializer().serializeToString(document);
    this.documentUrl = URL.createObjectURL(
      new Blob([serialized], { type: "application/xhtml+xml" }),
    );
    this.frame.title = `Generated EPUB preview — ${chapter.title}, source page ${chapter.source_page}`;
    this.frame.src = this.documentUrl;
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

  releasePageUrls() {
    if (this.documentUrl) URL.revokeObjectURL(this.documentUrl);
    for (const url of this.pageAssetUrls) URL.revokeObjectURL(url);
    this.documentUrl = null;
    this.pageAssetUrls = [];
  }
}

function resolveEpubPath(chapterHref, resourceHref) {
  try {
    const chapter = new URL(chapterHref, PREVIEW_ORIGIN);
    const resource = new URL(resourceHref, chapter);
    if (resource.origin !== new URL(PREVIEW_ORIGIN).origin) return "";
    return resource.pathname.replace(/^\/OEBPS\//, "");
  } catch {
    return "";
  }
}
