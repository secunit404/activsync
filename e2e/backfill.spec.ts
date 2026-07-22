import { test, expect } from "./fixtures";

// Same "connect Hevy through the real Settings form" setup as
// e2e/hevy-queue.spec.ts. This spec's "Connect Hevy" step writes to the
// shared e2e server/DB, so it needs the same isolation: playwright.config.ts
// matches this file only from the dedicated `hevy-backfill` project, which
// depends on `hevy-queue` and therefore always runs after it — strictly
// serial, never racing it or the three main viewport projects (which
// `testIgnore` this file). One end-to-end pass is what the coverage
// requirement asks for. Unlike hevy-queue.spec.ts's Skip flow, this spec
// never clicks Import, so it never mutates `hevy_workouts` — Preview is
// read-only.
test("locked row navigates to the mapping editor and back", async ({ page }) => {
  // Guarded, not unconditional, even though ordering is now deterministic
  // (hevy-queue always runs and connects first): this keeps the assertion
  // correct if `hevy_api_key` is ever seeded directly, without coupling
  // this spec to hevy-queue.spec.ts's internals.
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
  // Backfill is inline on the hub — there is no separate route to visit.
  await page.goto("/hevy");
  await page.getByRole("button", { name: "Preview" }).click();

  // Tick an importable row first: the scan AND this selection have to survive
  // opening the mapping editor, which is what living on the hub buys — the
  // card is never unmounted.
  // Exclude the header's "Select all importable" — it ticks every row, so
  // the count would not be 1. (Same trap as e2e/bulk-select.spec.ts.)
  const importable = page
    .getByRole("checkbox", { name: /^Select (?!all importable$)/ })
    .first();
  await importable.check();
  await expect(page.getByRole("button", { name: /^Import 1 selected$/ })).toBeVisible();

  await page.getByRole("link", { name: /^Map/ }).first().click();
  await expect(page).toHaveURL(/\/hevy\/mapping\//);
  // The hub is still mounted behind the editor. Asserted with a CSS locator,
  // not getByRole: Radix marks background content aria-hidden while a modal
  // is open, so the button is correctly absent from the accessibility tree
  // even though it is still in the DOM.
  await expect(page.locator("button", { hasText: /^Import 1 selected$/ })).toBeAttached();

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page).toHaveURL(/\/hevy$/);

  // Back on a populated screen, not a reset form.
  await expect(page.getByRole("button", { name: /^Import 1 selected$/ })).toBeVisible();
});
