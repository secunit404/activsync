import { test, expect } from "./fixtures";

// The dev-mock seed suffixes every sample title with "#<variant>" (see
// dev_seed.py) so there is no bare "Morning run" row — only "Morning run
// #1", "#2", "#3", scattered across different months. "Morning run #1" is
// the single occurrence that lands on page 1 (the newest page) in the
// current-month batch, so it's targeted by its exact name rather than the
// brief's bare-substring regex, which would strict-mode-fail once more than
// one suffixed match is visible on the same page.
const ACTIVITY_NAME = "Morning run #1";

test("detail presents as a centered modal on desktop", async ({ page }) => {
  test.skip(test.info().project.name === "mobile", "desktop/tablet only");
  await page.getByRole("link", { name: ACTIVITY_NAME, exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  const box = await dialog.boundingBox();
  expect(box!.width).toBeLessThan(700);
});

test("detail presents as a full-screen cover on mobile", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  // Both the desktop table row and the mobile card render the same title as
  // a real `<Link>` (see ActivityCard's docstring) — the two layouts are
  // switched by CSS only, so both exist in the DOM at every viewport. A
  // `display:none` element drops out of the accessibility tree, so
  // `getByRole("link", ...)` naturally resolves to just the visible one;
  // `getByText` would not (it matches raw DOM text regardless of
  // visibility) and hits a strict-mode violation here.
  await page.getByRole("link", { name: ACTIVITY_NAME, exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  const box = await dialog.boundingBox();
  const viewport = page.viewportSize()!;
  expect(box!.width).toBeGreaterThanOrEqual(viewport.width - 1);
});
