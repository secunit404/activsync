import type { Page } from "@playwright/test";

/** Rewrites `/api/v1/app` so the page sees these services as disconnected. */
export async function reportBroken(page: Page, services: Array<"garmin" | "strava">) {
  await page.route("**/api/v1/app", async (route) => {
    const response = await route.fetch();
    const state = await response.json();
    for (const service of services) {
      state.connections[service].connected = false;
    }
    state.connections.broken = services;
    await route.fulfill({ response, json: state });
  });
}
