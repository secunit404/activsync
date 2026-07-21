// e2e/smoke.spec.ts
import { test, expect } from "./fixtures";

test("activities page renders seeded data", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Activities" })).toBeVisible();
  await expect(page.getByText("Morning run")).toBeVisible();
});
