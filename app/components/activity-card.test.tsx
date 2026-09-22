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
  detail: {
    ...baseDetail,
    duration: "48:00",
    totalVolume: "8,420 kg",
  },
};

function renderCard(activity: Activity, props: Partial<{ selected: boolean; onToggle: (id: number) => void }> = {}) {
  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: (
          <ActivityCard
            activity={activity}
            selected={props.selected ?? false}
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

test("renders the title, status pill and metadata line", () => {
  renderCard(run);
  expect(screen.getByText("Morning run")).toBeInTheDocument();
  expect(screen.getByText("PENDING")).toBeInTheDocument();
  expect(screen.getByText("21 Jul · 06:42 · RUNNING")).toBeInTheDocument();
});

test("renders the stat line joining only the metrics that are present", () => {
  renderCard(run);
  expect(screen.getByText("52:14 · 10.2 km · 156 bpm")).toBeInTheDocument();
});

test("omits missing stat segments instead of showing an em dash", () => {
  renderCard(lift);
  expect(screen.getByText("48:00 · 8,420 kg")).toBeInTheDocument();
});

test("shows via Hevy in the metadata line instead of the type when hevyBadge is set", () => {
  renderCard(lift);
  expect(screen.getByText("21 Jul · 06:42 · via Hevy")).toBeInTheDocument();
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
  await userEvent.click(screen.getByText("52:14 · 10.2 km · 156 bpm"));
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
