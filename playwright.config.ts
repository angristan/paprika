import { defineConfig, devices } from "@playwright/test";

const localOrigin = "http://127.0.0.1:8787";

export default defineConfig({
  testDir: "./scripts/e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI
    ? [["line"], ["html", { open: "never", outputFolder: "target/playwright-report" }]]
    : "list",
  outputDir: "target/playwright-results",
  use: {
    baseURL: localOrigin,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "firefox",
      use: { ...devices["Desktop Firefox"] },
    },
    {
      name: "webkit",
      use: { ...devices["Desktop Safari"] },
    },
  ],
  webServer: {
    command: "bun run wrangler dev --ip 127.0.0.1 --port 8787",
    url: localOrigin,
    reuseExistingServer: false,
    stdout: "pipe",
    stderr: "pipe",
    timeout: 120_000,
  },
});
