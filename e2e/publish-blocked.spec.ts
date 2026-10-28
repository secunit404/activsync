import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { reportBroken } from "./broken-connections";

// The strava-publish project connects Strava before these run; the outage is
// faked on the wire so a shared connection is never torn down mid-run.
test.beforeEach(async ({ page }) => {
  await reportBroken(page, ["strava"]);
  await page.reload();
});

function pendingRow(page: Page) {
  return page
    .locator('tr:visible, [data-testid="activities-cards"] > div:visible')
    .filter({ hasText: "PENDING" })
    .first();
}

test("the bulk dock says why Publish is blocked and links to the fix", async ({ page }) => {
  await pendingRow(page).getByRole("checkbox").check();
  const bar = page.getByTestId("bulk-action-bar");
  const publish = bar.getByRole("button", { name: /^Publish \d/ }).locator("visible=true");

  await expect(publish).toBeDisabled();
  await expect(publish).toHaveAccessibleDescription("Reconnect Strava to publish.");
  const reconnect = bar.getByRole("link", { name: "Reconnect Strava" });
  await expect(reconnect).toBeVisible();

  await reconnect.click();
  await expect(page).toHaveURL(/\/settings#connections$/);
});

test("the detail overlay says why Publish is blocked", async ({ page }) => {
  await pendingRow(page).getByRole("link").first().click();
  const dialog = page.getByRole("dialog");

  await expect(dialog.getByRole("link", { name: "Reconnect Strava" })).toBeVisible();
  await expect(dialog.getByTestId("activity-status-line")).toContainText(
    "Publishes to Strava once Strava is reconnected.",
  );
});
