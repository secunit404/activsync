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

const pages = [
  { path: "/", title: "Activities" },
  { path: "/hevy", title: "Hevy" },
  { path: "/settings", title: "Settings" },
];

for (const { path, title } of pages) {
  test(`${title} names itself in the tab title and has no wordmark eyebrow`, async ({ page }) => {
    await page.goto(path);
    const heading = page.getByRole("heading", { level: 1, name: title });
    await expect(heading).toBeVisible();
    await expect(page).toHaveTitle(`${title} - ActivSync`);
    await expect(page.locator("header").getByLabel("ActivSync")).toHaveCount(0);
  });

  // Phones have no rail, so the page header carries the logo mark, on the
  // heading's own row rather than a row above it.
  test(`${title} shows the logo beside its heading on phones only`, async ({ page }) => {
    await page.goto(path);
    const heading = page.getByRole("heading", { level: 1, name: title });
    const logo = page.locator("header").getByRole("img", { name: "ActivSync" });
    if (test.info().project.name !== "mobile") {
      await expect(logo).toBeHidden();
      return;
    }
    await expect(logo).toBeVisible();
    await expect(heading).toBeVisible();
    const centre = (box: { y: number; height: number }) => box.y + box.height / 2;
    await expect
      .poll(async () => {
        const [logoBox, headingBox] = [await logo.boundingBox(), await heading.boundingBox()];
        if (!logoBox || !headingBox) return false;
        return (
          logoBox.width >= 28 &&
          logoBox.width <= 32 &&
          logoBox.x < headingBox.x &&
          Math.abs(centre(logoBox) - centre(headingBox)) <= 2
        );
      })
      .toBe(true);
  });
}
