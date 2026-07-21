import { test, expect } from "./fixtures";

test("selecting rows reveals the bulk bar", async ({ page }) => {
  await page.getByRole("checkbox").first().check();
  await expect(page.getByTestId("bulk-action-bar")).toBeVisible();
  await expect(page.getByTestId("bulk-action-bar")).toContainText("1");
});

test("mobile bulk bar replaces the tab bar", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  await page.getByRole("checkbox").first().check();
  await expect(page.getByTestId("bulk-action-bar")).toBeVisible();
  await expect(page.getByTestId("app-tab-bar")).toBeHidden();
});

test("clearing the selection hides the bar and restores the tab bar", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  await page.getByRole("checkbox").first().check();
  await expect(page.getByTestId("bulk-action-bar")).toBeVisible();

  await page.getByTestId("bulk-action-bar").getByRole("button", { name: "Clear" }).click();

  await expect(page.getByTestId("bulk-action-bar")).toBeHidden();
  await expect(page.getByTestId("app-tab-bar")).toBeVisible();
});
