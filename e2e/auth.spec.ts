import { expect, test } from "@playwright/test";

import {
  BASE_URL,
  CSRF_HEADERS,
  DEV_PASSWORD,
  OWNER_STATE,
  SIGNED_OUT,
  signIn,
} from "./auth";

// Runs alone and in order (see playwright.config.ts): the claim test drops
// the shared server's owner for a moment, and every other project's session
// depends on that owner.
test.describe.configure({ mode: "serial" });
test.use({ storageState: SIGNED_OUT });

test("a signed-out visit goes to login and comes back after signing in", async ({ page }) => {
  await page.goto("/hevy");
  await expect(page).toHaveURL(/\/login\?next=%2Fhevy$/);

  await page.getByLabel("Password").fill(DEV_PASSWORD);
  await page.getByRole("button", { name: "Sign in" }).click();

  await expect(page).toHaveURL(/\/hevy$/);
});

test("a wrong password is refused with one generic message", async ({ page }) => {
  await page.goto("/login");
  await page.getByLabel("Password").fill("not the dev password at all");
  await page.getByRole("button", { name: "Sign in" }).click();

  await expect(page.getByRole("alert")).toHaveText("Wrong password.");
  await expect(page).toHaveURL(/\/login$/);
});

test("signing out returns to login and ends the session", async ({ page }) => {
  await signIn(page, "/settings");
  await expect(page.getByRole("heading", { name: "Access" })).toBeVisible();

  await page.getByRole("button", { name: "Sign out" }).last().click();
  await expect(page).toHaveURL(/\/login$/);

  await page.goto("/settings");
  await expect(page).toHaveURL(/\/login\?next=%2Fsettings$/);
});

test("an unclaimed install walks the claim step", async ({ browser, page }) => {
  const owner = await browser.newContext({ baseURL: BASE_URL, storageState: OWNER_STATE });
  const unclaim = await owner.request.post("/api/v1/dev/unclaim", { headers: CSRF_HEADERS });
  expect(unclaim.ok()).toBeTruthy();
  const { setupCode } = (await unclaim.json()) as { setupCode: string };

  try {
    await page.goto("/");
    await expect(page).toHaveURL(/\/setup$/);
    await page.getByLabel("Setup code").fill(setupCode);
    await page.getByLabel("Password", { exact: true }).fill(DEV_PASSWORD);
    await page.getByLabel("Repeat password").fill(DEV_PASSWORD);
    await page.getByRole("button", { name: "Claim ActivSync" }).click();

    await expect(page.getByRole("heading", { name: "You’re all set" })).toBeVisible();
  } finally {
    const status = await owner.request.get("/api/v1/auth/status");
    if (!((await status.json()) as { ownerClaimed: boolean }).ownerClaimed) {
      await owner.request.post("/api/v1/auth/claim", {
        headers: CSRF_HEADERS,
        data: { setupCode, password: DEV_PASSWORD },
      });
    }
    await owner.close();
  }
});
