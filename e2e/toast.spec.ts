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

// Tick a row that can actually be excluded, rather than "the first
// checkbox". Every project in this suite shares one server and DB, and the
// first test here excludes a row — so by the second test that row is already
// excluded, and Exclude is (correctly) disabled for it. Selecting a PENDING
// row keeps each test independent of what ran before it.
//
// This used to pass by accident: Exclude was enabled regardless of status,
// the second click 409'd, and an *error* toast satisfied assertions that
// only check a toast appeared and can be closed.
async function selectPendingRow(page: import("@playwright/test").Page) {
  // `:visible` because a CSS locator, unlike a role query, would otherwise
  // match the display:none copy of the layout this viewport hides.
  const row = page
    .locator('tr:visible, [data-testid="activities-cards"] > div:visible')
    .filter({ hasText: "PENDING" })
    .first();
  await row.getByRole("checkbox").check();
  await expect(
    bulkActionBar(page).getByRole("button", { name: /^Exclude [1-9]/ }),
  ).toBeEnabled();
}

test("excluding an activity shows a success toast", async ({ page }) => {
  await selectPendingRow(page);
  await bulkActionBar(page).getByRole("button", { name: /exclude/i }).click();
  await expect(toastRegion(page)).toContainText(/exclude/i);
});

test("toast can be dismissed manually", async ({ page }) => {
  await selectPendingRow(page);
  await bulkActionBar(page).getByRole("button", { name: /exclude/i }).click();
  await page.getByRole("button", { name: /close/i }).click();
  await expect(toastRegion(page)).toBeHidden();
});
