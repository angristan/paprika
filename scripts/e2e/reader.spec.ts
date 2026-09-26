import { expect, test } from "@playwright/test";

test("paginates long EPUB content without clipping its final paragraph", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(async () => {
    const previewModule = "/epub-preview.js";
    const { EpubPreview } = await import(previewModule);
    const frame = document.querySelector<HTMLIFrameElement>("#preview-frame")!;
    frame.hidden = false;
    frame.style.width = "300px";
    frame.style.height = "360px";
    document.querySelector<HTMLElement>("#sheet")!.hidden = true;
    const reader = new EpubPreview(frame);
    (window as any).reader = reader;
    const paragraphs = Array.from({ length: 20 }, (_, index) =>
      `<p id="paragraph-${index}">Paragraph ${index + 1}. A reader should be able to turn pages while the text remains readable, without scrolling inside the preview.</p>`,
    ).join("");
    reader.setData({
      assets: [],
      stylesheet: "body { font: 16px/1.5 serif; } p { margin: 0 0 12px; }",
      chapters: [{
        href: "text/first.xhtml", title: "Long chapter", source_page: 1,
        xhtml: `<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Long chapter</title></head><body><main>${paragraphs}<p id="end">End of chapter.</p></main></body></html>`,
      }],
    }, []);
    await new Promise<void>((resolve) => reader.show(0, resolve));
  });

  expect(await page.evaluate(() => (window as any).reader.readerPageCount)).toBeGreaterThan(1);
  const firstPage = await page.frameLocator("#preview-frame").locator("#paragraph-0").boundingBox();
  const frame = await page.locator("#preview-frame").boundingBox();
  expect(firstPage!.y).toBeGreaterThanOrEqual(frame!.y);
  expect(firstPage!.y + firstPage!.height).toBeLessThanOrEqual(frame!.y + frame!.height);

  await page.evaluate(() => {
    const reader = (window as any).reader;
    for (let turn = 0; turn < 100 && reader.turnPage(1); turn++);
  });
  const lastPage = await page.frameLocator("#preview-frame").locator("#end").boundingBox();
  expect(lastPage!.x).toBeGreaterThanOrEqual(frame!.x);
  expect(lastPage!.x + lastPage!.width).toBeLessThanOrEqual(frame!.x + frame!.width + 1);
  expect(lastPage!.y + lastPage!.height).toBeLessThanOrEqual(frame!.y + frame!.height + 1);
  expect(await page.evaluate(() => (window as any).reader.turnPage(1))).toBe(false);
  expect(await page.evaluate(() => (window as any).reader.turnPage(-1))).toBe(true);

  // Resizing must recalculate the page count and keep the current page in range.
  const oldCount = await page.evaluate(() => (window as any).reader.readerPageCount);
  await page.locator("#preview-frame").evaluate((element) => {
    element.style.width = "240px";
    element.style.height = "280px";
  });
  await expect.poll(() => page.evaluate(() => (window as any).reader.readerPageCount)).toBeGreaterThan(oldCount);
  expect(await page.frameLocator("#preview-frame").locator("html").evaluate((element) =>
    element.scrollHeight <= element.clientHeight,
  )).toBe(true);

  await page.evaluate(() => (window as any).reader.clear());
});
