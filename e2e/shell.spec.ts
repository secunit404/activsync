import { test, expect } from "./fixtures";

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

test("nav moves between destinations", async ({ page }) => {
  await page.getByRole("link", { name: "Hevy" }).click();
  await expect(page).toHaveURL(/\/hevy$/);
});
