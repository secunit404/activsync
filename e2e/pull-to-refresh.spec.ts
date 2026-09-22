import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";

/**
 * A real finger drag. Playwright's `mouse.wheel` produces wheel events, which
 * this gesture deliberately ignores — only touch input reaches it, so the
 * gesture has to be synthesised through CDP.
 *
 * `Input.dispatchTouchEvent` rather than `Input.synthesizeScrollGesture`: the
 * latter is a compositor-level gesture that silently does nothing under
 * headless Linux, so the whole file passed on a developer's machine and failed
 * every CI run. Dispatching the touch points directly drives the same
 * `touchstart`/`touchmove`/`touchend` the component listens for, on every
 * platform.
 */
async function drag(page: Page, from: number, to: number) {
  const cdp = await page.context().newCDPSession(page);
  const x = 195;
  const steps = 10;
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x, y: from }],
  });
  for (let step = 1; step <= steps; step += 1) {
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x, y: from + ((to - from) * step) / steps }],
    });
  }
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchEnd",
    touchPoints: [],
  });
}

/**
 * Whether the component claimed the drag, which is the thing it actually
 * controls: claiming means calling `preventDefault` on the move, which is what
 * stops the page scrolling underneath the indicator. Asserting on that rather
 * than on `window.scrollY` keeps the test about our handler instead of about
 * Chromium's touch-to-scroll gesture recognition, which synthesised touch
 * points do not drive.
 *
 * Registered after the component's own listener, so it observes the flag the
 * component has by then set.
 */
async function recordGestureClaims(page: Page) {
  await page.evaluate(() => {
    const claims: boolean[] = [];
    (window as unknown as { __claims: boolean[] }).__claims = claims;
    window.addEventListener("touchmove", (event) => {
      claims.push(event.defaultPrevented);
    });
  });
}

async function expectGestureClaimed(page: Page, claimed: boolean) {
  const claims = await page.evaluate(
    () => (window as unknown as { __claims: boolean[] }).__claims,
  );
  // An empty list would pass either assertion below without the drag having
  // moved a finger at all, which is the failure mode this file just had.
  expect(claims.length).toBeGreaterThan(0);
  expect(claims).not.toContain(!claimed);
}

test.skip(() => test.info().project.name !== "mobile", "mobile only");

test("pulling down from the top refreshes", async ({ page }) => {
  const indicator = page.getByTestId("pull-to-refresh");
  await expect(indicator).toHaveCount(1);
  await expect(indicator).not.toHaveAttribute("data-armed", "true");
  await recordGestureClaims(page);

  const refreshed = page.waitForResponse(
    (r) => r.url().includes("/api/") && r.request().method() === "GET",
  );
  await drag(page, 200, 560);

  await refreshed;
  await expect.poll(() => indicator.getAttribute("data-refreshing")).toBeNull();
  await expectGestureClaimed(page, true);
  expect(await page.evaluate(() => window.scrollY)).toBe(0);
});

test("pulling while scrolled down scrolls instead of refreshing", async ({ page }) => {
  await page.mouse.wheel(0, 600);
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(0);
  await recordGestureClaims(page);

  await drag(page, 300, 560);

  await expect(page.getByTestId("pull-to-refresh")).not.toHaveAttribute("data-armed", "true");
  await expectGestureClaimed(page, false);
});
