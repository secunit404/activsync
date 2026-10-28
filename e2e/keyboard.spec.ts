import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";

const ACTIVITY_NAME = "Morning run #1";

function rowLink(page: Page) {
  return page.getByRole("link", { name: ACTIVITY_NAME, exact: true });
}

async function openWithKeyboard(page: Page) {
  await rowLink(page).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toBeVisible();
}

test("Skip to content is the first tab stop and lands in main", async ({ page }) => {
  const heading = page.getByRole("heading", { name: "Activities", level: 1 });
  const before = await heading.boundingBox();
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to content" });
  await expect(skip).toBeFocused();
  await expect(skip).toBeVisible();

  const after = await heading.boundingBox();
  expect(after!.y).toBe(before!.y);
  const chip = (await skip.boundingBox())!;
  const overlapsHeading =
    chip.x < after!.x + after!.width &&
    chip.x + chip.width > after!.x &&
    chip.y < after!.y + after!.height &&
    chip.y + chip.height > after!.y;
  expect(overlapsHeading).toBe(false);

  await page.keyboard.press("Enter");
  await expect(page.getByRole("main")).toBeFocused();
  await expect(page).toHaveURL(/\/\?status=all$/);
});

test("a focused row link draws a full-strength focus outline", async ({ page }) => {
  await page.keyboard.press("Tab");
  await rowLink(page).focus();
  const outline = await rowLink(page).evaluate((link) => {
    const style = getComputedStyle(link);
    return { style: style.outlineStyle, width: style.outlineWidth, color: style.outlineColor };
  });
  expect(outline).toEqual({ style: "solid", width: "2px", color: "rgb(242, 107, 56)" });
});

test("Escape returns focus to the row that opened the detail", async ({ page }) => {
  await openWithKeyboard(page);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(rowLink(page)).toBeFocused();
});

test("the close button returns focus to the row", async ({ page }) => {
  await openWithKeyboard(page);
  await page.getByRole("dialog").getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(rowLink(page)).toBeFocused();
});

test("browser Back returns focus to the row", async ({ page }) => {
  await openWithKeyboard(page);
  await page.goBack();
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(rowLink(page)).toBeFocused();
});

test("each row is one tab stop for its link, plus its checkbox", async ({ page }) => {
  const checkbox = page.getByRole("checkbox", { name: `Select ${ACTIVITY_NAME}`, exact: true });
  await checkbox.focus();
  await page.keyboard.press("Tab");
  await expect(rowLink(page)).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.locator(":focus")).toHaveAttribute("role", "checkbox");
  await expect(page.locator(":focus")).toHaveAccessibleName(/^Select (?!all activities$)/);
});

test("closing the bulk bar from the keyboard returns focus to the checkbox", async ({ page }) => {
  const checkbox = page.getByRole("checkbox", { name: `Select ${ACTIVITY_NAME}`, exact: true });
  await checkbox.focus();
  await page.keyboard.press("Space");
  await page.getByTestId("bulk-action-bar").locator("button:visible", { hasText: "Clear" }).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("bulk-action-bar")).toBeHidden();
  await expect(checkbox).toBeFocused();
});

// Safari does not focus a checkbox on click, so a mouse selection followed by
// Tab into the bar leaves the bar no origin to return focus to.
test("closing the bulk bar with no origin keeps focus on the list", async ({ page }) => {
  await page.getByRole("checkbox", { name: `Select ${ACTIVITY_NAME}`, exact: true }).click();
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  const bar = page.getByTestId("bulk-action-bar");
  await bar.locator("button:visible", { hasText: "Clear" }).focus();
  await page.keyboard.press("Enter");
  await expect(bar).toBeHidden();
  await expect(page.locator(":focus")).toHaveAttribute("data-activity-checkbox", /.*/);
  expect(await page.evaluate(() => document.activeElement === document.body)).toBe(false);
});
