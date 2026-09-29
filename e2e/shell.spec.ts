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
  // Scoped to the shell nav (rail or tab bar, whichever this viewport
  // shows), not a bare role/name query — Task 8's activity list renders
  // real <a> links for activity titles, and the seeded mock data includes
  // one literally titled "Forgot the watch (Hevy)", which also matches an
  // unscoped `getByRole("link", { name: "Hevy" })` substring query.
  const isMobile = test.info().project.name === "mobile";
  const nav = page.getByTestId(isMobile ? "app-tab-bar" : "app-rail");
  await nav.getByRole("link", { name: "Hevy" }).click();
  await expect(page).toHaveURL(/\/hevy$/);
});
