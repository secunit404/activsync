import { test, expect } from "./fixtures";

test("the Access card fits its width without clipping", async ({ page }) => {
  await page.goto("/settings");
  const card = page.locator("#access");
  await expect(card.getByRole("list", { name: "Signed-in sessions" })).toBeVisible();

  const { scrollWidth, clientWidth } = await card.evaluate((element) => ({
    scrollWidth: element.scrollWidth,
    clientWidth: element.clientWidth,
  }));
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth);
});
