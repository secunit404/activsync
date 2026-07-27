import { test, expect } from "./fixtures";

// The seeded e2e DB has no `hevy_api_key`, so Hevy starts disconnected (see
// `src/activsync/dev_seed.py` — it seeds Hevy *workouts* unconditionally,
// but never the API key that gates `/hevy`'s connected view). Reaching the
// sync queue's Skip flow therefore requires connecting Hevy first, through
// the real Settings form — `save_hevy_credentials` accepts any non-empty
// key in mock mode (`dev_mock.MockHevyClient`), so this is a legitimate
// user path, not a backdoor into the seeded data.
//
// This spec performs real, persisted mutations (connecting Hevy, then
// skipping "Ring circuit (Hevy)") against the shared e2e server/DB every
// project points at. Isolation is enforced by playwright.config.ts, not by
// a project-name check here: this file is matched only by the dedicated
// `hevy-queue` project, which runs alone — after the setup-* chain
// confirms onboarding is stable, before `hevy-backfill`, and before the
// three main viewport projects even start (they `testIgnore` this file
// entirely). One end-to-end pass is what the coverage requirement asks
// for, and running it under any other project would either duplicate the
// mutation or race it against e2e/backfill.spec.ts's own Hevy-connect step.
test("skip: cancelling leaves the workout queued, confirming skips it — via the real dialog, not window.confirm", async ({
  page,
}) => {
  await page.goto("/settings");
  await page
    .getByTestId("connection-status-hevy")
    .getByRole("button", { name: "Connect" })
    .click();
  await page.getByLabel("Hevy API key").fill("mock-hevy-key");
  await page.getByRole("button", { name: "Connect Hevy" }).click();
  await expect(page.getByText("Hevy connected.")).toBeVisible();

  await page.goto("/hevy");
  await expect(page.getByText("Ring circuit (Hevy)")).toBeVisible();

  // Scoped to the one row this test acts on. Skip is offered by every queue
  // row that has a decision pending — the seeded queue also holds an
  // awaiting-match workout ("Leg day (Hevy)") with its own Skip — so a
  // page-wide locator matches more than one button and is ambiguous about
  // which workout is being skipped. The row survives the skip: it moves into
  // the Skipped group, still carrying this title.
  const row = page.getByRole("listitem").filter({ hasText: "Ring circuit (Hevy)" });

  // Cancel: the previously-native `window.confirm()` would have hung
  // Playwright's CDP browser here — this dialog does not.
  await row.getByRole("button", { name: "Skip", exact: true }).click();
  const dialog = page.getByRole("alertdialog", { name: "Skip this workout?" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveAccessibleDescription("Skip Ring circuit (Hevy)?");

  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText("Ring circuit (Hevy)")).toBeVisible();
  await expect(row.getByRole("button", { name: "Unskip" })).not.toBeVisible();

  // Confirm: the destructive action actually runs.
  await row.getByRole("button", { name: "Skip", exact: true }).click();
  await page
    .getByRole("alertdialog", { name: "Skip this workout?" })
    .getByRole("button", { name: "Skip", exact: true })
    .click();

  await expect(page.getByText(/^Skipped/)).toBeVisible();
  await expect(page.getByRole("alertdialog")).toBeHidden();
  await expect(row.getByRole("button", { name: "Unskip" })).toBeVisible();
  await expect(row.getByRole("button", { name: "Skip", exact: true })).not.toBeVisible();
});
