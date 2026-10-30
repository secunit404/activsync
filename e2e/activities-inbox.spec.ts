import { test, expect } from "./fixtures";

test("the activities page opens on what needs review", async ({ page }) => {
  await page.goto("/");

  await expect(page).toHaveURL(/[?&]status=review/);
  await expect(page.getByRole("radio", { name: /Needs review/ })).toBeChecked();
  // `:visible` because the hidden layout (table or cards) carries the same pill.
  await expect(page.locator(':text-is("HELD"):visible').first()).toBeVisible();
});

test("an explicit All survives a reload", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("radio", { name: /^All/ }).click();
  await expect(page).toHaveURL(/[?&]status=all/);

  await page.reload();

  await expect(page.getByRole("radio", { name: /^All/ })).toBeChecked();
});
