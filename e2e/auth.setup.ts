import { expect, test as setup } from "@playwright/test";

import { OWNER_STATE, signIn } from "./auth";

setup("sign in once as the dev owner", async ({ page }) => {
  await signIn(page);
  await expect(page.getByRole("heading", { name: "Activities" })).toBeVisible();
  await page.context().storageState({ path: OWNER_STATE });
});
