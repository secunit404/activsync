import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test, vi } from "vitest";

import { ActivitiesTable } from "./activities-table";
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
  garminActivityId: 999,
  activityType: "strength_training",
  title: "Push Day",
  description: "",
  startTime: "2026-07-21 18:10:00",
  publishStatus: "published",
  stravaActivityId: 55,
  holdReason: null,
  startDateDisplay: "21 Jul",
  startMonthYearDisplay: "July 2026",
  startClockDisplay: "18:10",
  garminUrl: "https://connect.garmin.com/modern/activity/999",
  stravaUrl: "https://www.strava.com/activities/55",
  hevyBadge: "replace",
  detail: {
    ...baseDetail,
    duration: "48:00",
    totalVolume: "8,420 kg",
  },
};

function renderTable({
  activities = [run],
  selected = new Set<number>(),
  onToggle = vi.fn(),
  onToggleAll = vi.fn(),
}: {
  activities?: Activity[];
  selected?: ReadonlySet<number>;
  onToggle?: (id: number) => void;
  onToggleAll?: () => void;
} = {}) {
  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: (
          <ActivitiesTable
            activities={activities}
            selected={selected}
            onToggle={onToggle}
            onToggleAll={onToggleAll}
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

test("renders one row per activity with a link to its detail", () => {
  renderTable();
  expect(screen.getByRole("row", { name: /Morning run/ })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /Morning run/ })).toHaveAttribute(
    "href",
    "/12345",
  );
});

test("checkbox toggles selection without navigating", async () => {
  const onToggle = vi.fn();
  const router = renderTable({ onToggle });
  await userEvent.click(screen.getByRole("checkbox", { name: /Morning run/ }));
  expect(onToggle).toHaveBeenCalledWith(12345);
  expect(router.state.location.pathname).toBe("/");
});

test("clicking the row (not the checkbox) navigates to the activity detail", async () => {
  renderTable();
  await userEvent.click(screen.getByText("PENDING"));
  expect(await screen.findByText("Detail view")).toBeInTheDocument();
});

test("renders type pill, status pill, time, distance and effort columns", () => {
  renderTable();
  const row = screen.getByRole("row", { name: /Morning run/ });
  expect(within(row).getByText("RUNNING")).toBeInTheDocument();
  expect(within(row).getByText("PENDING")).toBeInTheDocument();
  expect(within(row).getByText("52:14")).toBeInTheDocument();
  expect(within(row).getByText("10.2 km")).toBeInTheDocument();
  expect(within(row).getByText("156 bpm")).toBeInTheDocument();
});

test("shows an em dash for missing distance and falls back to volume for effort", () => {
  renderTable({ activities: [lift] });
  const row = screen.getByRole("row", { name: /Push Day/ });
  expect(within(row).getByText("—")).toBeInTheDocument();
  expect(within(row).getByText("8,420 kg")).toBeInTheDocument();
});

test("notes the Hevy source in the activity's subtitle", () => {
  renderTable({ activities: [lift] });
  expect(screen.getByText(/via Hevy/)).toBeInTheDocument();
});

test("the Effort column is hidden below the lg breakpoint", () => {
  renderTable();
  const header = screen.getByRole("columnheader", { name: "EFFORT" });
  expect(header).toHaveClass("hidden", "lg:table-cell");
});

test("tints the row background when selected", () => {
  renderTable({ selected: new Set([12345]) });
  expect(screen.getByRole("row", { name: /Morning run/ })).toHaveClass("bg-primary/5");
});

test("header checkbox reflects select-all state and calls onToggleAll", async () => {
  const onToggleAll = vi.fn();
  renderTable({
    activities: [run, lift],
    selected: new Set([12345, 999]),
    onToggleAll,
  });
  const headerCheckbox = screen.getByRole("checkbox", { name: "Select all activities" });
  expect(headerCheckbox).toHaveAttribute("data-state", "checked");
  await userEvent.click(headerCheckbox);
  expect(onToggleAll).toHaveBeenCalled();
});

test("header checkbox is indeterminate when only some rows are selected", () => {
  renderTable({ activities: [run, lift], selected: new Set([12345]) });
  const headerCheckbox = screen.getByRole("checkbox", { name: "Select all activities" });
  expect(headerCheckbox).toHaveAttribute("data-state", "indeterminate");
});
