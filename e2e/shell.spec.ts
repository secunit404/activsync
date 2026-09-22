import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";

/**
 * The tab bar ignores scrolling for a short window after arriving on a route,
 * because `<ScrollRestoration>` puts a page back at its saved offset and that
 * jump is not a gesture (see `navigationSettleMs` in app-tab-bar.tsx). Tests
 * that scroll as the *user* have to clear that window first — a real user
 * takes far longer than this to tap a tab and then start scrolling.
 */
async function settle(page: Page) {
  await page.waitForTimeout(400);
}

test("desktop shows the rail and no tab bar", async ({ page }) => {
  test.skip(test.info().project.name === "mobile", "desktop/tablet only");
  await expect(page.getByTestId("app-rail")).toBeVisible();
  await expect(page.getByTestId("app-tab-bar")).toBeHidden();
});

test("mobile shows the tab bar and no rail", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  await expect(page.getByTestId("app-tab-bar")).toBeVisible();
  await expect(page.getByTestId("app-rail")).toBeHidden();
});

// The unit tests assert the classes; this asserts what they render to, with a
// real stylesheet — that the row genuinely goes and a circle genuinely takes
// its place, rather than the two merely carrying the right class names.
test("the tab bar minimizes to the active tab on scroll, and comes back", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  const nav = page.getByTestId("app-tab-bar");
  const row = page.getByTestId("app-tab-bar-row");
  const minimized = page.getByTestId("app-tab-bar-minimized");
  await expect(row).toBeVisible();
  await expect(minimized).toBeHidden();

  await settle(page);
  await page.mouse.wheel(0, 600);

  await expect(nav).toHaveAttribute("data-collapsed", "true");
  await expect(row).toBeHidden();
  await expect(minimized).toBeVisible();

  // An icon-only circle in the bottom-right corner: as wide as it is tall,
  // and against the same right edge the row occupied.
  const box = (await minimized.boundingBox())!;
  expect(box.width).toBeCloseTo(box.height, 0);
  expect(page.viewportSize()!.width - (box.x + box.width)).toBeCloseTo(16, 0);

  // Tapping it expands — the other route back is scrolling to the top, which
  // the next test covers.
  await minimized.click();
  await expect(nav).not.toHaveAttribute("data-collapsed", "true");
  await expect(row).toBeVisible();
  await expect(minimized).toBeHidden();
});

// The whole reason for the crossfade: the morph it replaced dragged the icon
// backwards before it travelled forwards. Nothing may move during this.
test("the minimized circle does not travel while it appears", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  const minimized = page.getByTestId("app-tab-bar-minimized");

  await settle(page);
  await page.mouse.wheel(0, 600);

  const positions: number[] = [];
  for (let i = 0; i < 8; i++) {
    const box = await minimized.boundingBox();
    if (box) {
      positions.push(Math.round(box.x + box.width / 2));
    }
    await page.waitForTimeout(30);
  }

  expect(positions.length).toBeGreaterThan(4);
  expect(Math.max(...positions) - Math.min(...positions)).toBeLessThanOrEqual(4);
});

test("returning to the top of the page expands the tab bar", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  const nav = page.getByTestId("app-tab-bar");
  await settle(page);
  await page.mouse.wheel(0, 600);
  await expect(nav).toHaveAttribute("data-collapsed", "true");

  await page.mouse.wheel(0, -600);

  await expect(nav).not.toHaveAttribute("data-collapsed", "true");
});

// `<ScrollRestoration>` puts a route back at its saved offset on arrival, and
// that jump reaches the tab bar as one huge scroll event. It used to count as
// a downward gesture, so pressing Hevy landed you on Hevy with the bar
// already minimized.
test("arriving on a route with a restored scroll position leaves the bar expanded", async ({
  page,
}) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  const nav = page.getByTestId("app-tab-bar");
  const row = page.getByTestId("app-tab-bar-row");

  await row.getByRole("link", { name: "Hevy" }).click();
  await page.waitForURL(/\/hevy$/);
  await settle(page);
  await page.mouse.wheel(0, 500);
  await expect(nav).toHaveAttribute("data-collapsed", "true");
  const restored = await page.evaluate(() => window.scrollY);
  expect(restored).toBeGreaterThan(0);

  await page.getByTestId("app-tab-bar-minimized").click();
  await row.getByRole("link", { name: "Activities" }).click();
  await page.waitForURL((u) => u.pathname === "/");

  await row.getByRole("link", { name: "Hevy" }).click();
  await page.waitForURL(/\/hevy$/);

  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(restored);
  await expect(nav).not.toHaveAttribute("data-collapsed", "true");
  await expect(row).toBeVisible();
});

test("nav moves between destinations", async ({ page }) => {
  // Scoped to the shell nav (rail or tab bar, whichever this viewport
  // shows), not a bare role/name query — Task 8's activity list renders
  // real <a> links for activity titles, and the seeded mock data includes
  // one literally titled "Forgot the watch (Hevy)", which also matches an
  // unscoped `getByRole("link", { name: "Hevy" })` substring query.
  const isMobile = test.info().project.name === "mobile";
  const nav = page.getByTestId(isMobile ? "app-tab-bar" : "app-rail");
  await nav.getByRole("link", { name: "Hevy" }).click();
  await expect(page).toHaveURL(/\/hevy$/);
});
