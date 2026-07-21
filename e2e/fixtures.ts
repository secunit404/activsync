import { test as base, expect } from "@playwright/test";

export const test = base.extend<{ seeded: void }>({
  seeded: [async ({ page }, use) => {
    // /api/v1/app is fetched by app-layout.tsx for every route under the
    // rail/tab-bar shell, so it's a reliable "the app has mounted and
    // talked to the backend" signal regardless of which route body ("/"
    // currently renders a Task 5 placeholder) is showing.
    const appStateResponse = page.waitForResponse((r) =>
      r.url().includes("/api/v1/app")
    );
    await Promise.all([appStateResponse, page.goto("/")]);
    await use();
  }, { auto: true }],
});

export { expect };
