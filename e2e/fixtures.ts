import { test as base, expect } from "@playwright/test";

export const test = base.extend<{ seeded: void }>({
  seeded: [async ({ page }, use) => {
    // /api/v1/app is fetched by app-layout.tsx for every route under the
    // rail/tab-bar shell, so it's a reliable "the app has mounted and
    // talked to the backend" signal regardless of which route body is
    // showing. /api/v1/activities is fetched by the real activities.tsx
    // (Task 7) every time "/" loads — wait on both so later specs aren't
    // racing an unpopulated page.
    const appStateResponse = page.waitForResponse((r) =>
      r.url().includes("/api/v1/app")
    );
    const activitiesResponse = page.waitForResponse((r) =>
      r.url().includes("/api/v1/activities")
    );
    await Promise.all([appStateResponse, activitiesResponse, page.goto("/")]);
    await use();
  }, { auto: true }],
});

export { expect };
