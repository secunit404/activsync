import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createRoutesStub } from "react-router";
import { expect, test, vi } from "vitest";

import { BackfillRow, type BackfillItem } from "./backfill-row";

function item(overrides: Partial<BackfillItem> = {}): BackfillItem {
  return {
    hevyId: "w-1",
    title: "Old ring circuit",
    startTime: "2026-07-01T10:00:00Z",
    action: "needs_mapping",
    twinActivityId: null,
    garminUrl: null,
    stravaActivityId: null,
    stravaUrl: null,
    missingTemplateIds: ["tpl-dev-custom"],
    workout: {
      hevyId: "w-1",
      title: "Old ring circuit",
      startTime: "2026-07-01T10:00:00Z",
      endTime: "2026-07-01T11:00:00Z",
      notes: null,
      exercises: [],
      descriptionPreview: "Old ring circuit",
    },
    ...overrides,
  };
}

function renderRow(value: BackfillItem) {
  const Stub = createRoutesStub([
    {
      path: "/",
      Component: () => (
        <ul>
          <BackfillRow item={value} selected={false} onToggle={vi.fn()} />
        </ul>
      ),
    },
  ]);
  render(<Stub initialEntries={["/"]} />);
}

// The mapping editor is nested under backfill so the preview survives the
// round trip — see routes.ts.
test("a locked row with a linkable template offers an enabled Map action", () => {
  renderRow(item());
  const link = screen.getByRole("link", { name: /Map/ });
  expect(link).toHaveAttribute("href", "/hevy/mapping/tpl-dev-custom");
});

// The dead-end case: needs_mapping with no template id to link to. Same
// control, disabled — not a differently-styled second badge.
test("a locked row with no linkable template shows the same control, disabled", () => {
  renderRow(item({ missingTemplateIds: [] }));
  expect(screen.queryByRole("link", { name: /Map/ })).toBeNull();
  const button = screen.getByRole("button", { name: /Map/ });
  expect(button).toBeDisabled();
  expect(button).toHaveAttribute(
    "title",
    "This exercise has no linkable template — it can't be fixed from here.",
  );
});

test("an unlocked row shows its kind badge and no Map control", () => {
  renderRow(
    item({
      action: "linked_existing",
      twinActivityId: 42,
      garminUrl: "https://connect.garmin.com/modern/activity/42",
      stravaActivityId: 84,
      stravaUrl: "https://www.strava.com/activities/84",
      missingTemplateIds: [],
    }),
  );
  expect(screen.getByText("Match")).toBeVisible();
  expect(screen.getByRole("link", { name: /Garmin/ })).toHaveAttribute(
    "href",
    "https://connect.garmin.com/modern/activity/42",
  );
  expect(screen.getByRole("link", { name: /Strava/ })).toHaveAttribute(
    "href",
    "https://www.strava.com/activities/84",
  );
  expect(screen.getByText("On Strava")).toBeVisible();
  expect(screen.queryByRole("link", { name: /Map/ })).toBeNull();
  expect(screen.queryByRole("button", { name: /Map/ })).toBeNull();
});

test("a defensive already-tracked row is disabled and never labelled Create", () => {
  renderRow(item({ action: "already_tracked", twinActivityId: null }));

  expect(screen.getByRole("checkbox", { name: /already tracked/i })).toBeDisabled();
  expect(screen.getByText("Tracked")).toBeVisible();
  expect(screen.getByText(/already tracked in ActivSync/i)).toBeVisible();
  expect(screen.queryByText("Create")).not.toBeInTheDocument();
});

test("opens the workout preview from a backfill row", async () => {
  renderRow(item({ action: "passive", missingTemplateIds: [] }));

  await userEvent.click(screen.getByRole("button", { name: "Preview Old ring circuit" }));

  expect(screen.getByRole("dialog", { name: "Old ring circuit" })).toBeVisible();
  expect(screen.getByText("Exercises · 0")).toBeVisible();
});
