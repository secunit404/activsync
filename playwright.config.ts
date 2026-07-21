import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://127.0.0.1:8384",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  projects: [
    // e2e/setup.spec.ts drives the first-run wizard, which requires
    // flipping the shared dev-mock server's onboarding flag off and back on
    // via POST /api/v1/dev/onboarding-state (see settings_api_routes.py).
    // Every other spec assumes the server stays onboarded the whole run, so
    // these three run setup.spec.ts ALONE first — chained through
    // `dependencies` into strict serial order — before any of the three
    // main projects (which explicitly ignore setup.spec.ts) are allowed to
    // start. Without this, `fullyParallel` would race a bulk-select or
    // shell spec against the onboarding flag mid-flip.
    {
      name: "setup-desktop",
      testMatch: /setup\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
    },
    {
      name: "setup-tablet",
      testMatch: /setup\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 834, height: 1112 } },
      dependencies: ["setup-desktop"],
    },
    {
      name: "setup-mobile",
      testMatch: /setup\.spec\.ts$/,
      use: { ...devices["Pixel 7"] },
      dependencies: ["setup-tablet"],
    },
    {
      name: "desktop",
      testIgnore: /setup\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
      dependencies: ["setup-mobile"],
    },
    {
      name: "tablet",
      testIgnore: /setup\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 834, height: 1112 } },
      dependencies: ["setup-mobile"],
    },
    {
      name: "mobile",
      testIgnore: /setup\.spec\.ts$/,
      use: { ...devices["Pixel 7"] },
      dependencies: ["setup-mobile"],
    },
  ],
  webServer: {
    command: "npm run dev:e2e",
    url: "http://127.0.0.1:8384",
    reuseExistingServer: !process.env.CI,
    timeout: 240_000,
  },
});
