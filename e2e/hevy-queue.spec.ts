import { test, expect } from "./fixtures";

// The seeded e2e DB has no `hevy_api_key`, so Hevy starts disconnected (see
// `src/activsync/dev_seed.py` — it seeds Hevy *workouts* unconditionally,
// but never the API key that gates `/hevy`'s connected view). Reaching the
// sync queue's Skip flow therefore requires connecting Hevy first, through
// the real Settings form — `save_hevy_credentials` accepts any non-empty
// key in mock mode (`dev_mock.MockHevyClient`), so this is a legitimate
// user path, not a backdoor into the seeded data.
//
// This whole flow runs on the desktop project only. It performs a real,
// persisted mutation (skip) against the shared e2e server/DB that every
// viewport project points at; running it on tablet/mobile too would race
// three copies of the same mutation against the same "Ring circuit (Hevy)"
// row. One end-to-end pass is what the coverage requirement asks for.
test("skip: cancelling leaves the workout queued, confirming skips it — via the real dialog, not window.confirm", async ({
  page,
}) => {
  test.skip(test.info().project.name !== "desktop", "single pass — avoids racing the shared mutation across viewport projects");

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

  // Cancel: the previously-native `window.confirm()` would have hung
  // Playwright's CDP browser here — this dialog does not.
  await page.getByRole("button", { name: "Skip", exact: true }).click();
  const dialog = page.getByRole("alertdialog", { name: "Skip this workout?" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveAccessibleDescription("Skip Ring circuit (Hevy)?");

  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText("Ring circuit (Hevy)")).toBeVisible();
  await expect(page.getByRole("button", { name: "Unskip" })).not.toBeVisible();

  // Confirm: the destructive action actually runs.
  await page.getByRole("button", { name: "Skip", exact: true }).click();
  await page
    .getByRole("alertdialog", { name: "Skip this workout?" })
    .getByRole("button", { name: "Skip", exact: true })
    .click();

  await expect(page.getByText(/^Skipped/)).toBeVisible();
  await expect(page.getByRole("alertdialog")).toBeHidden();
  await expect(page.getByRole("button", { name: "Unskip" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Skip", exact: true })).not.toBeVisible();
});
