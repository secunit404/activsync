import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";

/**
 * A real finger drag. Playwright's `mouse.wheel` produces wheel events, which
 * this gesture deliberately ignores — only touch input reaches it, so the
 * gesture has to be synthesised through CDP.
 */
async function drag(page: Page, from: number, to: number) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Input.synthesizeScrollGesture", {
    x: 195,
    y: from,
    yDistance: to - from,
    speed: 800,
    gestureSourceType: "touch",
  });
}

test.skip(() => test.info().project.name !== "mobile", "mobile only");

test("pulling down from the top refreshes", async ({ page }) => {
  const indicator = page.getByTestId("pull-to-refresh");
  await expect(indicator).toHaveCount(1);
  await expect(indicator).not.toHaveAttribute("data-armed", "true");

  const refreshed = page.waitForResponse(
    (r) => r.url().includes("/api/") && r.request().method() === "GET",
  );
  await drag(page, 200, 560);

  await refreshed;
  await expect.poll(() => indicator.getAttribute("data-refreshing")).toBeNull();
  // The page must be left where it started, not scrolled by the gesture.
  expect(await page.evaluate(() => window.scrollY)).toBe(0);
});

test("pulling while scrolled down scrolls instead of refreshing", async ({ page }) => {
  await page.mouse.wheel(0, 600);
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(0);
  const before = await page.evaluate(() => window.scrollY);

  await drag(page, 300, 560);

  // The drag scrolled the page back up rather than being eaten as a pull.
  await expect(page.getByTestId("pull-to-refresh")).not.toHaveAttribute("data-armed", "true");
  expect(await page.evaluate(() => window.scrollY)).toBeLessThan(before);
});
