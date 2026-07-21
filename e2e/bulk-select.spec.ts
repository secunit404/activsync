import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";

// On desktop, `getByRole("checkbox").first()` is the header select-all
// checkbox, not a row — checking it selects every activity, so a loose
// `toContainText("1")` assertion passes just as well for "12 selected" or
// "Publish 12". Target a specific row's checkbox (its accessible name is
// "Select <title>", never "Select all activities") and assert the exact
// count string instead.
function firstRowCheckbox(page: Page) {
  return page.getByRole("checkbox", { name: /^Select (?!all activities$)/ }).first();
}

test("selecting rows reveals the bulk bar", async ({ page }) => {
  await firstRowCheckbox(page).check();
  await expect(page.getByTestId("bulk-action-bar")).toBeVisible();
  await expect(page.getByTestId("bulk-action-bar")).toContainText("1 selected");
});

test("mobile bulk bar replaces the tab bar", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  await firstRowCheckbox(page).check();
  await expect(page.getByTestId("bulk-action-bar")).toBeVisible();
  await expect(page.getByTestId("app-tab-bar")).toBeHidden();
});

test("clearing the selection hides the bar and restores the tab bar", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile only");
  await firstRowCheckbox(page).check();
  await expect(page.getByTestId("bulk-action-bar")).toBeVisible();

  await page.getByTestId("bulk-action-bar").getByRole("button", { name: "Clear" }).click();

  await expect(page.getByTestId("bulk-action-bar")).toBeHidden();
  await expect(page.getByTestId("app-tab-bar")).toBeVisible();
});
