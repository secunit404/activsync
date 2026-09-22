// e2e/smoke.spec.ts
import { test, expect } from "./fixtures";

test("activities page renders", async ({ page }, testInfo) => {
  // dev_seed.py suffixes every seeded title with "#<variant>" (e.g. "Morning
  // run #1", "Morning run #2") to keep samples distinct across months, so
  // more than one activity card matches a bare "Morning run" substring —
  // that's a Playwright strict-mode violation, not a missing-data bug.
  // "Morning run #1" (offset 0, variant 0) is deterministically seeded every
  // run, so it's the correct exact string to assert on.
  //
  // The table (desktop/tablet) and card list (mobile) are two parallel DOM
  // subtrees switched by CSS breakpoint only (never JS), so both exist in
  // the page at once — scope to the layout this viewport actually shows or
  // the text locator resolves to two elements and trips Playwright's strict
  // mode.
  await expect(page.getByRole("heading", { name: "Activities" })).toBeVisible();
  const container =
    testInfo.project.name === "mobile"
      ? page.getByTestId("activities-cards")
      : page.getByTestId("activities-table");
  await expect(container.getByText("Morning run #1")).toBeVisible();
});
