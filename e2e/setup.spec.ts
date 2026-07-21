import { test, expect } from "@playwright/test";

// This spec starts at /setup, not /, so it doesn't use the seeded
// `fixtures.ts` (which navigates to / and waits on /api/v1/activities) —
// import straight from @playwright/test instead.
//
// The shared dev-mock E2E server always boots already onboarded (see
// main.py's ACTIVSYNC_DEV_E2E_ONBOARDED, set by `npm run dev:e2e`), so on
// its own /setup shows the "you're all set" screen and POST /setup/garmin
// 409s — there is no way to reach the wizard's Garmin step against that
// state. The mock-only POST /api/v1/dev/onboarding-state endpoint
// (settings_api_routes.py) flips `initial_sync_done` off for the duration
// of this test and back on afterwards, and playwright.config.ts runs this
// file, alone, before any other spec (see the `setup-*` projects and their
// `dependencies` chain) so the flip never races another spec's assumption
// that the server stays onboarded.
//
// dev_mock.py already has a real MFA trigger for exactly this: any Garmin
// password other than the literal string "mfa" succeeds immediately, and
// "mfa" simulates Garmin demanding a verification code — so this spec uses
// that existing hook rather than inventing a new one.

async function resetOnboarding(request: import("@playwright/test").APIRequestContext) {
  const response = await request.post("/api/v1/dev/onboarding-state", {
    data: { complete: false },
  });
  expect(response.ok()).toBeTruthy();
}

async function restoreOnboarding(request: import("@playwright/test").APIRequestContext) {
  const response = await request.post("/api/v1/dev/onboarding-state", {
    data: { complete: true },
  });
  expect(response.ok()).toBeTruthy();
}

test("wizard reaches the MFA step and presents correctly", async ({ page, request }) => {
  await resetOnboarding(request);
  try {
    await page.goto("/setup");
    await page.getByLabel("Email").fill("athlete@example.com");
    await page.getByLabel("Password").fill("mfa");
    await page.getByRole("button", { name: /connect/i }).click();
    await expect(page.getByRole("dialog", { name: /verification/i })).toBeVisible();
    await expect(page.getByRole("textbox", { name: /digit 1/i })).toBeFocused();
  } finally {
    await restoreOnboarding(request);
  }
});
