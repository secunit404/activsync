import { test, expect } from "./fixtures";

test("an unsaved edit survives a refresh, and the footer stays in view", async ({
  page,
}) => {
  await page.goto("/settings");
  const lookback = page.getByRole("spinbutton", {
    name: "Activity history window (days)",
  });
  await expect(lookback).toBeVisible();
  const original = await lookback.inputValue();
  const edited = String(Number(original) + 1);

  await lookback.fill(edited);
  const footer = page.getByTestId("settings-footer");
  await expect(footer).toHaveAttribute("data-dirty", "true");
  await expect(footer).toBeInViewport({ ratio: 1 });

  const refreshed = page.waitForResponse(
    (r) => r.url().includes("/api/v1/settings") && r.request().method() === "GET",
  );
  await page.getByRole("button", { name: "Sync Garmin" }).click();
  await refreshed;

  await expect(lookback).toHaveValue(edited);
  await expect(footer).toHaveAttribute("data-dirty", "true");

  await page.screenshot({
    path: `test-results/settings-drafts-${test.info().project.name}.png`,
  });

  await page.getByRole("button", { name: "Discard" }).click();
  await expect(lookback).toHaveValue(original);
  await expect(footer).toHaveAttribute("data-dirty", "false");
});
