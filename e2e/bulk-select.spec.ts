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

// Selecting an already-excluded activity alongside eligible ones used to
// leave Exclude enabled, and the server's 409 for the excluded one surfaced
// as a failed action. Both actions now report the eligible subset, which can
// never exceed the raw selection.
test("bulk actions report only the rows they can act on", async ({ page }) => {
  // Select-all lives in the desktop/tablet table header; the mobile card
  // layout has no equivalent, so there is no way to build a mixed selection
  // in one click there.
  test.skip(test.info().project.name === "mobile", "no select-all on mobile");

  await page.getByRole("checkbox", { name: "Select all activities" }).check();

  const bar = page.getByTestId("bulk-action-bar");
  await expect(bar).toBeVisible();

  const selected = Number(
    (await bar.getByText(/\d+ selected/).first().innerText()).replace(/\D/g, ""),
  );
  const excludable = Number(
    (await bar.getByRole("button", { name: /^Exclude \d+$/ }).first().innerText())
      .replace(/\D/g, ""),
  );

  expect(selected).toBeGreaterThan(0);
  expect(excludable).toBeLessThanOrEqual(selected);
  // The mock data seeds an excluded activity, so the subset is a strict
  // subset here — proving the filter actually applies rather than passing
  // trivially when the two happen to be equal.
  expect(excludable).toBeLessThan(selected);
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
