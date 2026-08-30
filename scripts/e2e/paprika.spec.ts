import { expect, test, type Page } from "@playwright/test";

function pdfFixture(pageTexts: string[]): Buffer {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Kids [${pageTexts.map((_, index) => `${index + 3} 0 R`).join(" ")}] /Count ${pageTexts.length} >>`,
    ...pageTexts.map((_, index) => {
      const contentId = pageTexts.length + 3 + index;
      const fontId = pageTexts.length * 2 + 3;
      return `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 ${fontId} 0 R >> >> /Contents ${contentId} 0 R >>`;
    }),
    ...pageTexts.map((text) => {
      const escaped = text.replaceAll("\\", "\\\\").replaceAll("(", "\\(").replaceAll(")", "\\)");
      const stream = `BT /F1 18 Tf 72 720 Td (${escaped}) Tj ET`;
      return `<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}\nendstream`;
    }),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];

  let pdf = "%PDF-1.4\n";
  const offsets = [0];
  for (const [index, object] of objects.entries()) {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  }
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  pdf += offsets.slice(1).map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(pdf);
}

async function selectPdf(page: Page, texts = ["Paprika local conversion test"]) {
  await page.locator("#source-file").setInputFiles({
    name: "fixture.pdf",
    mimeType: "application/pdf",
    buffer: pdfFixture(texts),
  });
  await expect(page.locator("#file-name")).toHaveText("fixture.pdf");
  await expect(page.locator("#preview-open")).toHaveAttribute("href", /^blob:/);
}

test.beforeEach(async ({ page }) => {
  await page.goto("/");
});

test("keeps the conversion workbench usable at 320 CSS pixels", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 844 });
  await expect(page.locator("h1")).toBeVisible();
  await expect(page.locator("#source-file")).toBeEnabled();
  await expect(page.locator("#convert")).toBeVisible();
  const metrics = await page.evaluate(() => {
    const clientWidth = document.documentElement.clientWidth;
    const overflowers = Array.from(document.querySelectorAll("body *"))
      .map((element) => {
        const rect = element.getBoundingClientRect();
        return {
          element: `${element.tagName.toLowerCase()}${element.id ? `#${element.id}` : ""}${element.classList.length ? `.${Array.from(element.classList).join(".")}` : ""}`,
          left: Math.round(rect.left),
          right: Math.round(rect.right),
          width: Math.round(rect.width),
        };
      })
      .filter(({ left, right }) => left < -1 || right > clientWidth + 1)
      .slice(0, 12);
    return {
      clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
      overflowers,
    };
  });
  expect(
    metrics.scrollWidth,
    `overflowing elements: ${JSON.stringify(metrics.overflowers)}`,
  ).toBeLessThanOrEqual(metrics.clientWidth + 1);
});

test("remains usable at 200 percent zoom", async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 900 });
  await page.evaluate(() => {
    document.documentElement.style.zoom = "2";
  });
  await expect(page.locator("#source-file")).toBeEnabled();
  await expect(page.locator("#convert")).toBeVisible();
  const metrics = await page.locator("body").evaluate((body) => ({
    clientWidth: body.clientWidth,
    scrollWidth: body.scrollWidth,
  }));
  expect(metrics.scrollWidth).toBeLessThanOrEqual(metrics.clientWidth + 1);
});

test("converts and downloads an EPUB with report metadata", async ({ page }) => {
  await selectPdf(page);
  await page.locator("#book-title").fill("Cross-browser fixture");
  await page.locator("#book-language").fill("fr-FR");
  await page.locator("#convert").click();

  await expect(page.locator(".status-label")).toHaveText("Ready", { timeout: 120_000 });
  await expect(page.locator("#source-page-count")).toHaveText("1");
  await expect(page.locator("#text-page-count")).toHaveText("1");
  await expect(page.locator("#warning-count")).toHaveText("No warnings");
  await expect(page.getByText("No conversion warnings.", { exact: true })).toHaveCount(0);
  const readyColumns = await page.locator(".workbench-body").evaluate((element) =>
    getComputedStyle(element).gridTemplateColumns.trim().split(/\s+/).length,
  );
  expect(readyColumns).toBe(3);
  await expect(page.locator("#download")).toBeVisible();
  await expect(page.locator("#preview-output")).toBeEnabled();
  await expect(page.locator("#preview-stage")).toHaveAttribute("data-preview", "output");
  await expect(page.locator("#preview-boundary")).toBeHidden();
  await expect(page.locator("#preview-position")).toContainText("scroll to read");
  await expect(page.locator("#app-shell")).toHaveAttribute("data-flow", "success");
  await expect(page.locator("#ready-summary")).toBeVisible();
  await expect(page.locator(".source-fields")).toBeHidden();

  await page.setViewportSize({ width: 320, height: 844 });
  await expect(page.locator("#ready-summary")).toBeVisible();
  await expect(page.locator("#ready-file")).toHaveText("fixture.pdf");
  await expect(page.locator(".source-fields")).toBeHidden();
  await page.locator("#edit-settings").click();
  await expect(page.locator(".source-fields")).toBeVisible();

  const downloadPromise = page.waitForEvent("download");
  await page.locator("#download").click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe("fixture.paprika.epub");
});

test("cancels a job and converts again with a fresh worker", async ({ page }) => {
  await selectPdf(page, Array.from({ length: 80 }, (_, index) => `Synthetic page ${index + 1}`));
  await page.locator("#convert").click();
  await expect(page.locator("#cancel")).toBeVisible();
  await expect(page.locator("#app-shell")).toHaveAttribute("data-flow", "working");
  await expect(page.locator("#preview-stage")).toHaveAttribute("data-preview", "working");
  await expect(page.locator(".task-promise")).toContainText("Reading and reflowing");
  await page.locator("#cancel").click();
  await expect(page.locator(".status-label")).toHaveText("Canceled");
  await expect(page.locator("#app-shell")).toHaveAttribute("data-flow", "ready");
  await expect(page.locator("#convert")).toBeEnabled();

  await page.locator("#convert").click();
  await expect(page.locator(".status-label")).toHaveText("Ready", { timeout: 120_000 });
  await expect(page.locator("#source-page-count")).toHaveText("80");
  await expect(page.locator("#ready-meta")).toHaveText("80 source pages");
  await expect(page.locator("#preview-limit")).toContainText("Preview capped at 12 pages");
  await expect(page.locator("#preview-limit")).toContainText("complete download");
});

test("surfaces conversion warnings before download", async ({ page }) => {
  await selectPdf(page, [""]);
  await page.locator("#convert").click();

  await expect(page.locator(".status-label")).toHaveText("Ready", { timeout: 120_000 });
  await expect(page.locator("#warning-count")).toContainText("warning");
  await expect(page.locator("#warning-groups")).toContainText(/text layer|selectable text/i);
});

test("renders raster PDF output and exposes a local preview", async ({ page }) => {
  await selectPdf(page);
  await page.locator("#format").selectOption("pdf");
  await page.locator("#convert").click();

  await expect(page.locator(".status-label")).toHaveText("Ready", { timeout: 120_000 });
  await expect(page.locator("#download")).toHaveAttribute("download", "fixture.paprika.pdf");
  await expect(page.locator("#preview-open")).toHaveAttribute("href", /^blob:/);
  await expect(page.locator("#preview-stage")).toHaveAttribute("data-preview", "pdf");
  await expect(page.locator("#preview-boundary")).toBeVisible();
});

test("shows safe diagnostics and recovers from invalid input", async ({ page }) => {
  await page.locator("#source-file").setInputFiles({
    name: "broken.pdf",
    mimeType: "application/pdf",
    buffer: Buffer.from("%PDF-not-a-document"),
  });
  await page.locator("#convert").click();
  await expect(page.locator(".status-label")).toHaveText("Could not convert", { timeout: 30_000 });
  await expect(page.locator("#app-shell")).toHaveAttribute("data-flow", "error");
  await expect(page.locator("#preview-stage")).toHaveAttribute("data-preview", "error");
  await expect(page.locator("#diagnostic-text")).toContainText("Document names and contents: omitted");
  await expect(page.locator("#diagnostic-text")).not.toContainText("broken.pdf");

  await selectPdf(page);
  await page.locator("#convert").click();
  await expect(page.locator(".status-label")).toHaveText("Ready", { timeout: 120_000 });
});
