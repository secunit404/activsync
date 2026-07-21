import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router";
import { expect, test, vi } from "vitest";

import type { ActivitiesPage, ActivityQuery, AppState } from "@/lib/api";
import { HomeView } from "./home";

const state: AppState = {
  name: "ActivSync",
  version: "0.1.0",
  development: true,
  setup: { complete: true, step: null },
  connections: {
    garmin: {
      connected: true,
      status: "Connected",
      meta: "last synced 2 min ago",
      email: "athlete@example.com",
    },
    strava: { connected: true, status: "Connected", meta: "" },
    broken: [],
  },
  hevy: { enabled: true, connected: true, status: "Connected" },
  catchUpReport: null,
  update: {
    latest: null,
    available: false,
    repoUrl: "https://github.com/secunit404/activsync",
    releaseUrl: "https://github.com/secunit404/activsync/releases/latest",
  },
};

const query: ActivityQuery = {
  sort: "newest",
  status: null,
  page: 1,
  pageSize: 20,
};

const activities: ActivitiesPage = {
  items: [
    {
      garminActivityId: 42,
      activityType: "running",
      title: "Morning run",
      description: "Easy loop before work.",
      startTime: "2026-07-19 07:00:00",
      publishStatus: "published",
      stravaActivityId: 99,
      holdReason: null,
      startDateDisplay: "19 Jul",
      startMonthYearDisplay: "July 2026",
      startClockDisplay: "09:00",
      garminUrl: "https://connect.garmin.com/modern/activity/42",
      stravaUrl: "https://www.strava.com/activities/99",
      hevyBadge: null,
      detail: {
        description: "Easy loop before work.",
        distance: "7.85 km",
        duration: "42m 00s",
        movingTime: "40m 30s",
        elapsedTime: "45m 00s",
        pace: "5:21 /km",
        speed: "11.2 km/h",
        elevGain: "120 m",
        elevLoss: "110 m",
        calories: "420",
        avgHr: "142 bpm",
        maxHr: "168 bpm",
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
      },
    },
  ],
  sort: "newest",
  status: null,
  counts: {
    pending: 0,
    held: 0,
    published: 1,
    missing: 0,
    excluded: 0,
  },
  weekTotal: {
    seconds: 0,
    display: "0m",
  },
  pagination: {
    page: 1,
    pageSize: 20,
    pageCount: 1,
    totalCount: 1,
    firstItem: 1,
    lastItem: 1,
  },
};

const idleActionState = { isPending: false, action: undefined };
const actionResult = {
  message: "Action completed",
  severity: "success" as const,
  publishedCount: 0,
  failedCount: 0,
  blockedCount: 0,
};

test("renders the read-only activities dashboard", () => {
  render(
    <MemoryRouter>
      <HomeView
        state={state}
        activities={activities}
        query={query}
        actionState={idleActionState}
        onActivityAction={vi.fn().mockResolvedValue(actionResult)}
        onQueryChange={vi.fn()}
      />
    </MemoryRouter>,
  );

  expect(screen.getByRole("heading", { name: "Activities" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Morning run" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Settings" })).toHaveAttribute(
    "href",
    "/settings",
  );
  expect(screen.getByText("Mock data")).toBeInTheDocument();
  expect(screen.getByText("Garmin ready")).toBeInTheDocument();
  expect(screen.getByText("7.85 km")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Strava" })).toHaveAttribute(
    "href",
    "https://www.strava.com/activities/99",
  );
  expect(screen.getByText("ActivSync v0.1.0")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "GitHub" })).toHaveAttribute(
    "href",
    "https://github.com/secunit404/activsync",
  );
});

test("renders the reconnect report and Hevy recovery queue", () => {
  const client = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <HomeView
          state={{
            ...state,
            catchUpReport: { new: 4, held: 3, linked: 1, days: 12 },
          }}
          activities={activities}
          hevyQueue={{
            enabled: true,
            inFlight: [],
            skipped: [],
            problems: [
              {
                hevyId: "workout-1",
                title: "Upper body",
                status: "needs_mapping",
                error: "Map Bulgarian Ring Row",
                startDisplay: "19 Jul 10:00",
                needsMapping: true,
                hasOpenOperation: false,
                resyncable: false,
              },
            ],
            counts: { inFlight: 0, problems: 1, skipped: 0 },
          }}
          query={query}
          actionState={idleActionState}
          onActivityAction={vi.fn().mockResolvedValue(actionResult)}
          onQueryChange={vi.fn()}
          onDismissCatchUp={vi.fn().mockResolvedValue(undefined)}
        />
      </MemoryRouter>
    </QueryClientProvider>,
  );

  expect(screen.getByText("Reconnect catch-up complete")).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Hevy sync" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Upper body" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Map exercises" })).toHaveAttribute(
    "href",
    "/settings#hevy-mappings",
  );
  expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
});

test("reports activity filter changes and resets the page", () => {
  const onQueryChange = vi.fn();
  render(
    <MemoryRouter>
      <HomeView
        state={state}
        activities={activities}
        query={query}
        actionState={idleActionState}
        onActivityAction={vi.fn().mockResolvedValue(actionResult)}
        onQueryChange={onQueryChange}
      />
    </MemoryRouter>,
  );

  fireEvent.change(screen.getByRole("combobox", { name: "Filter activities" }), {
    target: { value: "held" },
  });

  expect(onQueryChange).toHaveBeenCalledWith({ status: "held", page: 1 });
});

test("reports next-page navigation", () => {
  const onQueryChange = vi.fn();
  render(
    <MemoryRouter>
      <HomeView
        state={state}
        activities={{
          ...activities,
          pagination: {
            ...activities.pagination,
            pageCount: 2,
            totalCount: 21,
            lastItem: 20,
          },
        }}
        query={query}
        actionState={idleActionState}
        onActivityAction={vi.fn().mockResolvedValue(actionResult)}
        onQueryChange={onQueryChange}
      />
    </MemoryRouter>,
  );

  fireEvent.click(screen.getByRole("link", { name: "Go to next page" }));

  expect(onQueryChange).toHaveBeenCalledWith({ page: 2 });
});

test("publishes selected activities in bulk", async () => {
  const onActivityAction = vi.fn().mockResolvedValue(actionResult);
  render(
    <MemoryRouter>
      <HomeView
        state={state}
        activities={{
          ...activities,
          items: [
            {
              ...activities.items[0],
              publishStatus: "pending",
              stravaActivityId: null,
              stravaUrl: null,
            },
          ],
          counts: { ...activities.counts, pending: 1, published: 0 },
        }}
        query={query}
        actionState={idleActionState}
        onActivityAction={onActivityAction}
        onQueryChange={vi.fn()}
      />
    </MemoryRouter>,
  );

  fireEvent.click(screen.getByRole("button", { name: "Select multiple" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Select all on page" }));
  fireEvent.click(screen.getByRole("button", { name: "Publish selected" }));

  await waitFor(() =>
    expect(onActivityAction).toHaveBeenCalledWith({
      type: "publish-many",
      activityIds: [42],
    }),
  );
});

test("edits an activity and keeps feedback inside its details dialog", async () => {
  const onActivityAction = vi.fn().mockResolvedValue({
    ...actionResult,
    message: "Saved changes to Morning run",
  });
  render(
    <MemoryRouter>
      <HomeView
        state={state}
        activities={activities}
        query={query}
        actionState={idleActionState}
        onActivityAction={onActivityAction}
        onQueryChange={vi.fn()}
      />
    </MemoryRouter>,
  );

  fireEvent.click(screen.getByRole("button", { name: "Details" }));
  const dialog = screen.getByRole("dialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "Edit" }));
  fireEvent.change(within(dialog).getByLabelText("Title"), {
    target: { value: "Lunch run" },
  });
  fireEvent.change(within(dialog).getByLabelText("Description"), {
    target: { value: "Updated route" },
  });
  fireEvent.click(within(dialog).getByRole("button", { name: "Save changes" }));

  await waitFor(() =>
    expect(onActivityAction).toHaveBeenCalledWith({
      type: "edit",
      activityId: 42,
      title: "Lunch run",
      description: "Updated route",
    }),
  );
  expect(within(dialog).getByText("Changes saved")).toBeInTheDocument();
  expect(
    within(dialog).getByText("Saved changes to Morning run"),
  ).toBeInTheDocument();
});
