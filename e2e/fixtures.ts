import { test as base, expect } from "@playwright/test";

export const test = base.extend<{ seeded: void }>({
  seeded: [async ({ page }, use) => {
    const activitiesResponse = page.waitForResponse((r) =>
      r.url().includes("/api/v1/activities")
    );
    await Promise.all([activitiesResponse, page.goto("/")]);
    await use();
  }, { auto: true }],
});

export { expect };
