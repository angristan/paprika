import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const localOrigin = "http://127.0.0.1:8787";

test.beforeEach(async ({ page }) => {
  const offOriginRequests: string[] = [];
  page.on("request", (request) => {
    if (!request.url().startsWith(localOrigin)) offOriginRequests.push(request.url());
  });
  await page.goto("/");
  expect(offOriginRequests, "the static app must not contact third parties").toEqual([]);
});

test("serves the task-first local conversion workbench", async ({ page }) => {
  await expect(page).toHaveTitle(/Paprika/);
  await expect(page.getByRole("heading", { level: 1 })).toContainText("Keep the book");
  await expect(page.locator(".task-promise")).toContainText("PDF in. Reflowable EPUB out.");
  await expect(page.getByRole("region", { name: "Convert" })).toBeVisible();
  await expect(page.locator(".folio-spine")).toBeVisible();
  await expect(page.locator("#app-shell")).toHaveAttribute("data-flow", "empty");
  await expect(page.getByRole("button", { name: "Make EPUB" })).toBeDisabled();
  await expect(page.locator("#route-source")).toHaveAttribute("aria-current", "step");
  await expect(page.getByText(/Document bytes stay local/)).toBeVisible();
  await expect(page.locator("#preview-frame")).toHaveAttribute("sandbox", "allow-same-origin");
});

test("has no automated WCAG A or AA violations", async ({ page }) => {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(results.violations).toEqual([]);
});

test("switches to the explicit raster fallback", async ({ page }) => {
  await page.getByLabel("Format").selectOption("pdf");
  await expect(page.getByRole("button", { name: "Make raster PDF" })).toBeDisabled();
  await expect(page.getByLabel("Raster layout")).toBeVisible();
  await expect(page.getByText("Experimental · image-only output.")).toBeVisible();
});

test("uses the documented local PDF preview boundary", async ({ page }) => {
  await page.locator("#source-file").setInputFiles({
    name: "local.pdf",
    mimeType: "application/pdf",
    buffer: Buffer.from("%PDF-1.4\n%%EOF\n"),
  });
  await expect(page.locator("#preview-frame")).not.toHaveAttribute("sandbox", /.+/);
  await expect(page.locator("#preview-open")).toHaveAttribute("rel", /noopener/);
  await expect(page.locator("#preview-boundary")).toBeVisible();
  await expect(page.locator("#preview-stage")).toHaveAttribute("data-preview", "source");
  await expect(page.locator(".task-promise")).toContainText("Source loaded locally");
  await expect(page.getByRole("button", { name: "Make EPUB" })).toBeEnabled();
});

test("rejects a non-PDF before conversion", async ({ page }) => {
  await page.locator("#source-file").setInputFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("not a PDF"),
  });
  await expect(page.getByText("Wrong file", { exact: true })).toBeVisible();
  await expect(page.getByText("Paprika currently accepts PDF documents only.")).toBeVisible();
  await expect(page.locator("#route-source")).toHaveAttribute("data-state", "error");
  await expect(page.locator("#route-compose")).toHaveAttribute("data-state", "pending");
});

test("ships restrictive headers and both licenses", async ({ request }) => {
  const home = await request.get("/");
  expect(home.ok()).toBeTruthy();
  expect(home.headers()["content-security-policy"]).toContain("connect-src 'self'");
  expect(home.headers()["referrer-policy"]).toBe("no-referrer");
  expect(home.headers()["x-content-type-options"]).toBe("nosniff");

  for (const license of ["LICENSE-APACHE", "LICENSE-MIT"]) {
    const response = await request.get(`/${license}`);
    expect(response.ok(), `${license} must be included in the deploy`).toBeTruthy();
  }
});
