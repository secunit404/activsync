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
    // e2e/hevy-queue.spec.ts, e2e/backfill.spec.ts, and
    // e2e/strava-publish.spec.ts each perform real, persisted mutations
    // against connection state (`hevy_api_key`, `strava_tokens`,
    // `garmin_credentials_verified`) and/or the activities table that every
    // other project's tests implicitly assume is stable for the run:
    //   - e2e/toast.spec.ts and e2e/smoke.spec.ts pin specific rows/titles
    //     ("Morning run #1") and rely on Strava staying disconnected
    //     (`publishDisabled`) — a concurrent publish or Garmin catch-up
    //     sync could reorder/relabel rows out from under them.
    //   - e2e/bulk-select.spec.ts selects "the first row", which a
    //     concurrent publish must not be allowed to reorder mid-test.
    //   - e2e/hevy-queue.spec.ts and e2e/backfill.spec.ts both connect
    //     Hevy through the same real Settings form and read/write the same
    //     "Ring circuit (Hevy)" queue row.
    // None of that is safe under `fullyParallel`, so — same pattern as the
    // setup-* chain above — these three run alone, in strict serial order,
    // after onboarding is confirmed stable (`setup-mobile`) and before any
    // of the three main viewport projects (which now ignore all three
    // files) are allowed to start.
    {
      name: "hevy-queue",
      testMatch: /hevy-queue\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
      dependencies: ["setup-mobile"],
    },
    {
      name: "hevy-backfill",
      testMatch: /backfill\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
      dependencies: ["hevy-queue"],
    },
    {
      name: "strava-publish",
      testMatch: /strava-publish\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
      dependencies: ["hevy-backfill"],
    },
    // e2e/toast.spec.ts drives its toasts through bulk Exclude, which is a
    // real persisted mutation. The three viewport projects below run
    // concurrently with each other, so running this file in all three raced
    // it against itself: one worker excluded the row another had just read
    // as excludable, and Exclude — correctly — disables for a selection it
    // cannot act on. Neither test is viewport-specific (the mobile toast's
    // bottom offset is covered by bulk-select.spec.ts's tab-bar cases), so
    // this runs once, alone, in the same serial chain as the specs above.
    {
      name: "toast",
      testMatch: /toast\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
      dependencies: ["strava-publish"],
    },
    {
      name: "desktop",
      testIgnore: /(setup|hevy-queue|backfill|strava-publish|toast)\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
      dependencies: ["toast"],
    },
    {
      name: "tablet",
      testIgnore: /(setup|hevy-queue|backfill|strava-publish|toast)\.spec\.ts$/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 834, height: 1112 } },
      dependencies: ["toast"],
    },
    {
      name: "mobile",
      testIgnore: /(setup|hevy-queue|backfill|strava-publish|toast)\.spec\.ts$/,
      use: { ...devices["Pixel 7"] },
      dependencies: ["toast"],
    },
  ],
  webServer: {
    command: "npm run dev:e2e",
    url: "http://127.0.0.1:8384",
    reuseExistingServer: !process.env.CI,
    timeout: 240_000,
  },
});
