import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test, vi } from "vitest";

import { ActivityCard } from "./activity-card";
import type { Activity } from "@/lib/api";

const baseDetail: Activity["detail"] = {
  description: "",
  distance: "",
  duration: "",
  movingTime: "",
  elapsedTime: "",
  pace: "",
  speed: "",
  elevGain: "",
  elevLoss: "",
  calories: "",
  avgHr: "",
  maxHr: "",
  avgPower: "",
  maxPower: "",
  normPower: "",
  aerobicTe: "",
  anaerobicTe: "",
  trainingLoad: "",
  avgCadence: "",
  maxCadence: "",
  totalSets: "",
  totalReps: "",
  totalVolume: "",
};

const run: Activity = {
  garminActivityId: 12345,
  activityType: "running",
  title: "Morning run",
  description: "",
  startTime: "2026-07-21 06:42:00",
  publishStatus: "pending",
  stravaActivityId: null,
  holdReason: null,
  startDateDisplay: "21 Jul",
  startMonthYearDisplay: "July 2026",
  startClockDisplay: "06:42",
  garminUrl: "https://connect.garmin.com/modern/activity/12345",
  stravaUrl: null,
  hevyBadge: null,
  publishScheduledAt: null,
  publishPhase: null,
  detail: {
    ...baseDetail,
    distance: "10.2 km",
    duration: "52:14",
    avgHr: "156 bpm",
  },
};

const lift: Activity = {
  ...run,
  garminActivityId: 999,
  activityType: "strength_training",
  title: "Push Day",
  publishStatus: "published",
  hevyBadge: "replace",
  publishScheduledAt: null,
  publishPhase: null,
  detail: {
    ...baseDetail,
    duration: "48:00",
    totalVolume: "8,420 kg",
  },
};

function renderCard(activity: Activity, props: Partial<{ selected: boolean; publishing: boolean; onToggle: (id: number) => void }> = {}) {
  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: (
          <ActivityCard
            activity={activity}
            selected={props.selected ?? false}
            publishing={props.publishing ?? false}
            onToggle={props.onToggle ?? vi.fn()}
          />
        ),
      },
      { path: "/:id", element: <span>Detail view</span> },
    ],
    { initialEntries: ["/"] },
  );
  render(<RouterProvider router={router} />);
  return router;
}

function metaItems(text: string) {
  return Array.from(screen.getByText(text).parentElement?.children ?? []).map(
    (item) => item.textContent,
  );
}

test("renders the title, status pill and metadata line", () => {
  renderCard(run);
  expect(screen.getByText("Morning run")).toBeInTheDocument();
  expect(screen.getByText("PENDING")).toBeInTheDocument();
  expect(metaItems("21 Jul 06:42")).toEqual(["21 Jul 06:42", "RUNNING"]);
});

test("renders the stat line joining only the metrics that are present", () => {
  renderCard(run);
  expect(metaItems("52:14")).toEqual(["52:14", "10.2 km", "156 bpm"]);
});

test("omits missing stat segments instead of showing an em dash", () => {
  renderCard(lift);
  expect(metaItems("48:00")).toEqual(["48:00", "8,420 kg"]);
});

test("shows via Hevy in the metadata line instead of the type when hevyBadge is set", () => {
  renderCard(lift);
  expect(metaItems("21 Jul 06:42")).toEqual(["21 Jul 06:42", "via Hevy"]);
});

test("checkbox toggles selection without navigating", async () => {
  const onToggle = vi.fn();
  const router = renderCard(run, { onToggle });
  await userEvent.click(screen.getByRole("checkbox", { name: /Morning run/ }));
  expect(onToggle).toHaveBeenCalledWith(12345);
  expect(router.state.location.pathname).toBe("/");
});

test("clicking the card body (not the title link or checkbox) navigates to the activity detail", async () => {
  renderCard(run);
  await userEvent.click(screen.getByText("52:14"));
  expect(await screen.findByText("Detail view")).toBeInTheDocument();
});

test("clicking the title link navigates to the activity detail", () => {
  renderCard(run);
  expect(screen.getByRole("link", { name: "Morning run" })).toHaveAttribute(
    "href",
    "/12345",
  );
});

test("the card itself is not a link — only the title is (avoids accessible-name collisions elsewhere on the page)", () => {
  renderCard(lift);
  expect(screen.getAllByRole("link")).toHaveLength(1);
});

test("tints the card when selected", () => {
  renderCard(run, { selected: true });
  expect(screen.getByRole("checkbox", { name: /Morning run/ })).toHaveAttribute(
    "data-state",
    "checked",
  );
});

test("a publishing card hides its reason without removing it, so the card keeps its height", () => {
  renderCard({ ...run, publishStatus: "held", holdReason: "backlog" }, { publishing: true });
  expect(screen.getByText("PUBLISHING…")).toBeInTheDocument();
  const reason = screen.getByText("Older activity found after a reconnect");
  expect(reason).toHaveClass("invisible");
  expect(reason).toHaveAttribute("aria-hidden", "true");
});
