import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { reportBroken } from "./broken-connections";

const WCAG_AA = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

async function violations(page: Page) {
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG_AA).analyze();
  return violations.map((v) => ({
    id: v.id,
    impact: v.impact,
    targets: v.nodes.map((n) => n.target.join(" ")),
  }));
}

test("Activities has no WCAG AA violations", async ({ page }) => {
  await expect(page.getByRole("heading", { level: 1, name: "Activities" })).toBeVisible();
  expect(await violations(page)).toEqual([]);
});

test("Needs review has no WCAG AA violations", async ({ page }) => {
  await page.goto("/?status=review");
  await expect(page.getByRole("heading", { level: 1, name: "Activities" })).toBeVisible();
  expect(await violations(page)).toEqual([]);
});

test("the activity detail has no WCAG AA violations", async ({ page }) => {
  await page.getByRole("link", { name: "Morning run #1", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  expect(await violations(page)).toEqual([]);
});

test("the selection bar has no WCAG AA violations", async ({ page }) => {
  await page.getByRole("checkbox").nth(1).check();
  await expect(page.getByRole("region", { name: "Selected activities" })).toBeVisible();
  expect(await violations(page)).toEqual([]);
});

test("the merged reconnect banner has no WCAG AA violations", async ({ page }) => {
  await reportBroken(page, ["garmin", "strava"]);
  await page.goto("/?status=missing");
  await expect(page.getByText("Reconnect Garmin and Strava")).toBeVisible();
  await expect(page.getByRole("button", { name: /^Reconnect/ })).toHaveCount(2);
  await expect(page.getByRole("link", { name: /^Only missing/ })).toBeVisible();
  expect(await violations(page)).toEqual([]);
});
