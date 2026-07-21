// e2e/smoke.spec.ts
import { test, expect } from "./fixtures";

test("activities page renders seeded data", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Activities" })).toBeVisible();
  // dev_seed.py suffixes every seeded title with "#<variant>" (e.g. "Morning
  // run #1", "Morning run #2") to keep samples distinct across months, so
  // more than one activity card matches a bare "Morning run" substring —
  // that's a Playwright strict-mode violation, not a missing-data bug.
  // "Morning run #1" (offset 0, variant 0) is deterministically seeded every
  // run, so it's the correct exact string to assert on.
  await expect(page.getByText("Morning run #1")).toBeVisible();
});
