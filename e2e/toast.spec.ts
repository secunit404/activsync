import { test, expect } from "./fixtures";

// The brief's spec drives this via the bulk Publish button, but the e2e
// dev-mock seed (`dev_seed.py`) never sets `strava_tokens` — Strava is
// always "disconnected" in this environment (see `view.connection_status`),
// so `BulkActionBar`'s Publish button is permanently `publishDisabled` here,
// independent of which row is selected. Faking a Strava connection for e2e
// is out of scope for a toast restyle, so this exercises the same
// `useActivityActions` success-toast pipeline through Exclude instead — the
// one bulk action that is never gated on a connection (see
// `bulk-action-bar.tsx`'s `publishDisabled` doc comment).
function bulkActionBar(page: import("@playwright/test").Page) {
  return page.getByTestId("bulk-action-bar");
}

// `role="status"` alone is also ambiguous: `ui/spinner.tsx` already puts
// `role="status"` on every in-flight spinner (see its `aria-label="Loading"`),
// and the bulk action bar's own (disabled) Publish button shows one for as
// long as the shared `useActivityActions` mutation this Exclude click drives
// is pending. The toaster's root `<section>` is the one with sonner's
// default `containerAriaLabel`, "Notifications" — name on that to get past
// the spinner.
function toastRegion(page: import("@playwright/test").Page) {
  return page.getByRole("status", { name: /notifications/i });
}

test("excluding an activity shows a success toast", async ({ page }) => {
  await page.getByRole("checkbox").first().check();
  await bulkActionBar(page).getByRole("button", { name: /exclude/i }).click();
  await expect(toastRegion(page)).toContainText(/exclude/i);
});

test("toast can be dismissed manually", async ({ page }) => {
  await page.getByRole("checkbox").first().check();
  await bulkActionBar(page).getByRole("button", { name: /exclude/i }).click();
  await page.getByRole("button", { name: /close/i }).click();
  await expect(toastRegion(page)).toBeHidden();
});
