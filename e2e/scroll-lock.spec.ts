import { test, expect } from "./fixtures";

// Playwright hides scrollbars by default; this spec is about the gutter they take.
test.use({ launchOptions: { ignoreDefaultArgs: ["--hide-scrollbars"] } });

test("opening a modal does not shift the page sideways", async ({ page }) => {
  test.skip(test.info().project.name === "mobile", "desktop/tablet only");
  // Headless Chromium on macOS draws overlay scrollbars, which take no width
  // and so can't shift anything. Styling the scrollbar forces a classic one.
  await page.addStyleTag({ content: "::-webkit-scrollbar { width: 15px; }" });
  expect(
    await page.evaluate(() => window.innerWidth - document.documentElement.clientWidth),
  ).toBe(15);
  // A handle, not a role locator: an open dialog hides the page behind it
  // from the accessibility tree.
  const row = (await page
    .getByRole("link", { name: "Morning run #1", exact: true })
    .elementHandle())!;
  const before = (await row.boundingBox())!.x;

  await row.click();
  await expect(page.getByRole("dialog")).toBeVisible();

  expect((await row.boundingBox())!.x).toBe(before);
});
