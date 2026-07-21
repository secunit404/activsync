import { test, expect } from "./fixtures";

// The seeded e2e DB never sets `strava_tokens` or `garmin_credentials_verified`
// (see `src/activsync/dev_seed.py` — it inserts activity rows directly,
// bypassing the sync path entirely), so every publish control starts
// disabled and no other e2e spec exercises a successful publish. Both
// connections are reachable through the real Settings form in mock mode —
// no backdoor needed:
//   - Garmin: `POST /api/v1/settings/garmin/reconnect` accepts any password
//     other than the literal string "mfa" (`dev_mock.begin_login`), the
//     same hook e2e/setup.spec.ts already relies on.
//   - Strava: credentials are saved via `PUT
//     /api/v1/settings/strava-credentials`, then "Connect" does a real
//     `window.location.assign("/strava/connect")`. In mock mode
//     `FakeStravaClient.authorize_url` loops straight back to our own
//     `/strava/callback` with a placeholder code instead of bouncing to
//     strava.com (see `dev_mock.py`), so the whole OAuth round trip
//     completes locally and lands back on /settings — a real page
//     navigation Playwright follows like any other.
//
// "Saturday ride #2" (offset 0, variant 1 in dev_seed.py) is the seed's
// second-newest activity — always on page 1 — and, unlike "Morning run #1"
// (used by e2e/smoke.spec.ts and e2e/overlay.spec.ts), its exact suffixed
// title appears nowhere else in the 36-row seed, so selecting it can't
// collide with another spec's row or trip Playwright's strict mode.
//
// This spec mutates the same kind of global, persisted state as
// e2e/hevy-queue.spec.ts and e2e/backfill.spec.ts (connection state, plus
// the published activity's status) and needs the same isolation:
// playwright.config.ts matches this file only from the dedicated
// `strava-publish` project, which runs after `hevy-backfill` and before
// the three main viewport projects (which `testIgnore` it) even start.
test("connecting Strava in mock mode and publishing an activity succeeds end to end", async ({
  page,
}) => {
  await page.goto("/settings");

  // Garmin must also read as connected — `require_publish_connections`
  // (api_routes.py) 409s a publish unless both services are connected, even
  // though the bulk action bar's own `publishDisabled` only looks at Strava.
  // Both dialogs' triggers keep showing "Reconnect"/"Connect" the whole
  // time they're open (that text only flips once the mutation succeeds and
  // the connection query refetches), so the in-dialog buttons below are
  // scoped to the dialog itself — an unscoped query would strict-mode-fail
  // on trigger + in-dialog button sharing the same accessible name.
  const garminStatus = page.getByTestId("connection-status-garmin-connect");
  await garminStatus.getByRole("button", { name: "Reconnect" }).click();
  const garminDialog = page.getByRole("dialog", { name: "Manage Garmin" });
  await garminDialog.getByLabel("Garmin email").fill("athlete@example.com");
  await garminDialog.getByLabel("Garmin password").fill("mock-garmin-password");
  await garminDialog.getByRole("button", { name: "Reconnect", exact: true }).click();
  await expect(
    page.getByText("Garmin reconnected and catch-up sync started."),
  ).toBeVisible();

  const stravaStatus = page.getByTestId("connection-status-strava");
  await stravaStatus.getByRole("button", { name: "Connect" }).click();
  const stravaDialog = page.getByRole("dialog", { name: "Manage Strava" });
  await stravaDialog.getByLabel("Strava client ID").fill("mock-strava-client-id");
  await stravaDialog
    .getByLabel("Strava client secret")
    .fill("mock-strava-client-secret");
  await stravaDialog.getByRole("button", { name: "Save credentials" }).click();
  await expect(
    page.getByText("Strava credentials saved. Continue to authorization."),
  ).toBeVisible();

  await stravaDialog.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page).toHaveURL(/\/settings$/);
  await expect(stravaStatus.getByText("Connected", { exact: true })).toBeVisible();

  await page.goto("/");
  const row = page
    .getByTestId("activities-table")
    .locator("tr", { hasText: "Saturday ride #2" });
  await row.getByRole("checkbox", { name: "Select Saturday ride #2" }).check();

  const bulkActionBar = page.getByTestId("bulk-action-bar");
  await expect(bulkActionBar).toContainText("1 selected");
  await bulkActionBar.getByRole("button", { name: /^Publish/ }).click();

  const toastRegion = page.getByRole("status", { name: /notifications/i });
  await expect(toastRegion).toContainText(/Published 1 activity to Strava/i);
  await expect(row.getByText("PUBLISHED", { exact: true })).toBeVisible();
});
