import { test as base, expect } from "@playwright/test";

export const test = base.extend<{ seeded: void }>({
  seeded: [async ({ page }, use) => {
    await page.goto("/");
    await page.waitForResponse((r) => r.url().includes("/api/v1/activities"));
    await use();
  }, { auto: true }],
});

export { expect };
