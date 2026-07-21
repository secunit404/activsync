import { test, expect } from "./fixtures";

// Same "connect Hevy through the real Settings form" setup as
// e2e/hevy-queue.spec.ts, and the same desktop-only guard — this spec's
// "Connect Hevy" step writes to the shared e2e server/DB, and racing it
// against the other viewport projects (or hevy-queue.spec.ts's own connect
// step) buys nothing: one end-to-end pass is what the coverage requirement
// asks for. Unlike hevy-queue.spec.ts's Skip flow, this spec never clicks
// Import, so it never mutates `hevy_workouts` — Preview is read-only.
test("locked row navigates to the mapping editor and back", async ({ page }) => {
  test.skip(
    test.info().project.name !== "desktop",
    "single pass — avoids racing shared Hevy-connect state across viewport projects",
  );

  // Guarded, not unconditional: this spec and e2e/hevy-queue.spec.ts both
  // run desktop-only against the same shared server/DB, and both connect
  // Hevy through this same form — whichever runs first leaves it connected
  // for the other, so "Connect" may already have become "Manage" by the
  // time this runs.
  await page.goto("/settings");
  const hevyStatus = page.getByTestId("connection-status-hevy");
  await expect(hevyStatus).toBeVisible();
  const connectButton = hevyStatus.getByRole("button", { name: "Connect" });
  if (await connectButton.count()) {
    await connectButton.click();
    await page.getByLabel("Hevy API key").fill("mock-hevy-key");
    await page.getByRole("button", { name: "Connect Hevy" }).click();
    await expect(page.getByText("Hevy connected.")).toBeVisible();
  }

  // dev_mock.HEVY_DEV_BACKFILL_UNMAPPED_ID ("Old ring circuit (Hevy)") is a
  // fresh (never-tracked) workout using the same unmapped custom exercise as
  // the hub's mapping-queue demo, timestamped 5 days ago — inside the
  // screen's default 30-day lookback, so Preview surfaces it with no need to
  // change the "since" field first.
  await page.goto("/hevy/backfill");
  await page.getByRole("button", { name: "Preview" }).click();
  await page.getByRole("link", { name: /map/i }).first().click();
  await expect(page).toHaveURL(/\/hevy\/mapping\//);
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page).toHaveURL(/\/hevy\/backfill$/);
});
