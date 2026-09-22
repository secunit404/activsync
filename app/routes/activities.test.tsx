import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { Activity, ActivitiesPage, AppState } from "@/lib/api";
import { ActivitiesView } from "./activities";

const { publishActivities, excludeActivity, dismissCatchUpReport } = vi.hoisted(() => ({
  publishActivities: vi.fn(),
  excludeActivity: vi.fn(),
  dismissCatchUpReport: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, publishActivities, excludeActivity, dismissCatchUpReport };
});

// These spies are module-level, so call counts accumulate across tests
// unless they are reset — an assertion like "called once" would otherwise
// silently measure every preceding test too.
beforeEach(() => {
  vi.clearAllMocks();
});

// The stat strip renders two parallel layouts (tablet/desktop grid vs. the
// mobile single-card strip) switched by CSS breakpoint, not JS — per the
// project convention of never branching on window size in code (see
// ResponsiveOverlay). jsdom has no layout engine, so both subtrees are
// present in every test render; counts are kept distinct per status so
// getByText inside a scoped container is unambiguous.
const activities: ActivitiesPage = {
  items: [],
  sort: "newest",
  status: null,
  counts: {
    pending: 2,
    held: 5,
    published: 34,
    missing: 3,
    excluded: 1,
  },
  weekTotal: {
    seconds: 27720,
    display: "7h 42m",
  },
  pagination: {
    page: 1,
    pageSize: 20,
    pageCount: 3,
    totalCount: 42,
    firstItem: 1,
    lastItem: 20,
  },
};

const activityDetail: Activity["detail"] = {
  description: "",
  distance: "10.2 km",
  duration: "52:14",
  movingTime: "",
  elapsedTime: "",
  pace: "",
  speed: "",
  elevGain: "",
  elevLoss: "",
  calories: "",
  avgHr: "156 bpm",
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

const oneActivity: Activity = {
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
  detail: activityDetail,
};

const anotherActivity: Activity = {
  ...oneActivity,
  garminActivityId: 999,
  title: "Push Day",
};

function renderAt(
  path: string,
  data: ActivitiesPage = activities,
  appState?: Pick<AppState, "connections" | "catchUpReport">,
) {
  const queryClient = new QueryClient();
  const router = createMemoryRouter(
    [
      { path: "*", element: <ActivitiesView data={data} appState={appState} /> },
      { path: "/settings", element: <div>Settings page</div> },
    ],
    { initialEntries: [path] },
  );
  render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return router;
}

test("renders the page heading and subtitle", () => {
  renderAt("/");
  expect(screen.getByRole("heading", { name: "Activities" })).toBeInTheDocument();
  expect(
    screen.getByText("Review your Garmin sync history before it reaches Strava."),
  ).toBeInTheDocument();
});

test("renders the four stat tiles from the activities page data", () => {
  renderAt("/");
  const grid = screen.getByTestId("stat-tiles-grid");
  expect(within(grid).getByText("PENDING")).toBeInTheDocument();
  expect(within(grid).getByText("HELD")).toBeInTheDocument();
  expect(within(grid).getByText("PUBLISHED")).toBeInTheDocument();
  expect(within(grid).getByText("THIS WEEK")).toBeInTheDocument();
  expect(within(grid).getByText("2")).toBeInTheDocument();
  expect(within(grid).getByText("5")).toBeInTheDocument();
  expect(within(grid).getByText("34")).toBeInTheDocument();
  // weekTotal.display is rendered verbatim, never reformatted client-side.
  expect(within(grid).getByText("7h 42m")).toBeInTheDocument();
});

test("renders the mobile compact strip with the same data", () => {
  renderAt("/");
  const strip = screen.getByTestId("stat-tiles-strip");
  expect(within(strip).getByText("PENDING")).toBeInTheDocument();
  expect(within(strip).getByText("34")).toBeInTheDocument();
  expect(within(strip).getByText("7h 42m")).toBeInTheDocument();
});

test("renders the filter pills wired to the same counts", () => {
  renderAt("/");
  expect(
    screen.getByRole("group", { name: "Filter activities by status" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /^All/ })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /Missing/ })).toBeInTheDocument();
});

// Ported from the deleted home.test.tsx ("reports activity filter changes
// and resets the page"). ActivityFilterPills now owns the URL directly
// instead of calling back into an onQueryChange prop, so this asserts the
// resulting URL state rather than a spy call.
test("selecting a filter resets the page to 1", () => {
  const router = renderAt("/?status=published&page=4");

  fireEvent.click(screen.getByRole("button", { name: /Held/ }));

  const search = new URLSearchParams(router.state.location.search);
  expect(search.get("status")).toBe("held");
  expect(search.has("page")).toBe(false);
});

test("renders an empty state when there are no activities at all", () => {
  renderAt("/");
  expect(screen.getByText("No activities found")).toBeInTheDocument();
  // No status is applied (data.status is null) — the copy must say so is
  // an onboarding/fresh-install state, not imply a filter is hiding rows.
  expect(
    screen.getByText("Activities will appear here after the first Garmin sync."),
  ).toBeInTheDocument();
});

// A user who filters to Held and sees a bare "No activities found" would
// think their data vanished — the description must name the active filter
// so it reads as "nothing here yet for this filter", not "everything's gone".
test("distinguishes an empty filtered view from a genuinely empty history", () => {
  renderAt("/", { ...activities, status: "held", items: [] });
  expect(screen.getByText("No activities found")).toBeInTheDocument();
  expect(
    screen.getByText("There are no held activities in the current sync history."),
  ).toBeInTheDocument();
  expect(
    screen.queryByText("Activities will appear here after the first Garmin sync."),
  ).not.toBeInTheDocument();
});

test("renders an attention banner for each broken connection, naming each service", () => {
  renderAt("/", activities, {
    connections: {
      garmin: { connected: false, status: "", meta: "", email: "" },
      strava: { connected: false, status: "", meta: "" },
      broken: ["garmin", "strava"],
    },
    catchUpReport: null,
  });

  const alerts = screen.getAllByRole("alert");
  expect(alerts).toHaveLength(2);
  expect(alerts.some((alert) => /Garmin/.test(alert.textContent ?? ""))).toBe(true);
  expect(alerts.some((alert) => /Strava/.test(alert.textContent ?? ""))).toBe(true);
});

test("renders no attention banner when nothing is broken", () => {
  renderAt("/", activities, {
    connections: {
      garmin: { connected: true, status: "", meta: "", email: "" },
      strava: { connected: true, status: "", meta: "" },
      broken: [],
    },
    catchUpReport: null,
  });
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

test("clicking Reconnect on the attention banner navigates to Settings connections", async () => {
  const router = renderAt("/", activities, {
    connections: {
      garmin: { connected: true, status: "", meta: "", email: "" },
      strava: { connected: false, status: "", meta: "" },
      broken: ["strava"],
    },
    catchUpReport: null,
  });

  await userEvent.click(screen.getByRole("button", { name: /reconnect/i }));

  expect(router.state.location.pathname).toBe("/settings");
  expect(router.state.location.hash).toBe("#connections");
});

test("renders the catch-up report and dismisses it", async () => {
  dismissCatchUpReport.mockResolvedValue(undefined);
  renderAt("/", activities, {
    connections: { garmin: { connected: true, status: "", meta: "", email: "" }, strava: { connected: true, status: "", meta: "" }, broken: [] },
    catchUpReport: { new: 12, held: 4, linked: 8, days: 30 },
  });

  expect(screen.getByText(/Reconnect catch-up complete/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(dismissCatchUpReport).toHaveBeenCalledOnce();
});

test("renders no catch-up report when there is nothing to report", () => {
  renderAt("/", activities, {
    connections: { garmin: { connected: true, status: "", meta: "", email: "" }, strava: { connected: true, status: "", meta: "" }, broken: [] },
    catchUpReport: null,
  });
  expect(screen.queryByText(/Reconnect catch-up complete/)).not.toBeInTheDocument();
});

test("renders the pagination control from the pagination data", () => {
  renderAt("/");
  expect(screen.getByText("Page 1 of 3")).toBeInTheDocument();
});

test("renders a table row (desktop) and a card (mobile) for each activity", () => {
  renderAt("/", { ...activities, items: [oneActivity] });
  expect(
    within(screen.getByRole("table")).getByRole("row", { name: /Morning run/ }),
  ).toBeInTheDocument();
  // Both the table row and the mobile card render "Morning run" — the two
  // layouts are switched by CSS breakpoint only (never JS), so jsdom keeps
  // both subtrees in the DOM regardless of viewport, same as the stat strip.
  expect(screen.getAllByText("Morning run")).toHaveLength(2);
});

// Ported from the deleted home.test.tsx's "publishes selected activities in
// bulk" case, scoped to Task 9's real useSelection/BulkActionBar wiring
// rather than Task 8's local-state stand-in.
test("checking a row reveals the bulk action bar with the right count", async () => {
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] });
  expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument();

  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);

  const bar = screen.getByTestId("bulk-action-bar");
  expect(within(bar).getAllByText("1 selected").length).toBeGreaterThan(0);
});

// The now-deleted dashboard disabled Publish while a connection was broken;
// the redesigned BulkActionBar must keep doing so, or a user sees the
// AttentionBanner saying publishing is paused directly above an enabled
// Publish button. Exclude is a purely local write, unaffected by Strava
// being reachable, so it must stay enabled.
test("disables Publish (not Exclude) when Strava is broken", async () => {
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] }, {
    connections: {
      garmin: { connected: true, status: "", meta: "", email: "" },
      strava: { connected: false, status: "", meta: "" },
      broken: ["strava"],
    },
    catchUpReport: null,
  });
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);

  // Not /Publish/ — that also matches the "Published 10" filter pill.
  for (const button of screen.getAllByRole("button", { name: /Publish 1/ })) {
    expect(button).toBeDisabled();
  }
  for (const button of screen.getAllByRole("button", { name: /Exclude/ })) {
    expect(button).not.toBeDisabled();
  }
});

// A broken Garmin connection alone does not pause publishing (see
// AttentionBanner's copy: "Strava publishing continues for activities
// already synced").
test("keeps Publish enabled when only Garmin is broken", async () => {
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] }, {
    connections: {
      garmin: { connected: false, status: "", meta: "", email: "" },
      strava: { connected: true, status: "", meta: "" },
      broken: ["garmin"],
    },
    catchUpReport: null,
  });
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);

  // Not /Publish/ — that also matches the "Published 10" filter pill.
  for (const button of screen.getAllByRole("button", { name: /Publish 1/ })) {
    expect(button).not.toBeDisabled();
  }
});

test("selecting a filter clears the selection", async () => {
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] });
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);
  expect(screen.getAllByTestId("bulk-action-bar").length).toBeGreaterThan(0);

  fireEvent.click(screen.getByRole("button", { name: /Held/ }));

  expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument();
});

test("publishing the selection clears it and calls publishActivities with the selected ids", async () => {
  publishActivities.mockResolvedValue({
    message: "Published 1 activity to Strava",
    severity: "success",
    publishedCount: 1,
    failedCount: 0,
    blockedCount: 0,
  });
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] });
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);

  await userEvent.click(screen.getAllByRole("button", { name: /Publish 1/ })[0]);

  expect(publishActivities).toHaveBeenCalledWith([12345]);
  await waitFor(() =>
    expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument(),
  );
});

test("excluding the selection calls excludeActivity for each selected id and clears the selection", async () => {
  excludeActivity.mockResolvedValue({
    message: "Excluded",
    severity: "success",
    publishedCount: 0,
    failedCount: 0,
    blockedCount: 0,
  });
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] });
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Push Day/ })[0]);

  await userEvent.click(screen.getAllByRole("button", { name: /Exclude/ })[0]);

  await waitFor(() => expect(excludeActivity).toHaveBeenCalledTimes(2));
  expect(excludeActivity).toHaveBeenCalledWith(12345);
  expect(excludeActivity).toHaveBeenCalledWith(999);
  await waitFor(() =>
    expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument(),
  );
});

// The reported bug: selecting an already-excluded row alongside eligible
// ones left Exclude enabled, and the server answered 409 for the excluded
// one — surfacing to the user as "it didn't work". Exclude now acts on the
// eligible subset only, and says how many that is.
test("a mixed selection excludes only the eligible rows", async () => {
  excludeActivity.mockResolvedValue({
    message: "Excluded",
    severity: "success",
    publishedCount: 0,
    failedCount: 0,
    blockedCount: 0,
  });
  const alreadyExcluded: Activity = {
    ...oneActivity,
    garminActivityId: 777,
    title: "Already excluded",
    publishStatus: "excluded",
  };
  renderAt("/", { ...activities, items: [oneActivity, alreadyExcluded] });

  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Already excluded/ })[0]);

  // Two selected, but only one of them can actually be excluded.
  expect(screen.getAllByText("2 selected").length).toBeGreaterThan(0);
  await userEvent.click(screen.getAllByRole("button", { name: "Exclude 1" })[0]);

  await waitFor(() => expect(excludeActivity).toHaveBeenCalledTimes(1));
  expect(excludeActivity).toHaveBeenCalledWith(12345);
  expect(excludeActivity).not.toHaveBeenCalledWith(777);
});

test("Exclude is disabled when every selected row is already excluded", async () => {
  const alreadyExcluded: Activity = {
    ...oneActivity,
    garminActivityId: 777,
    title: "Already excluded",
    publishStatus: "excluded",
  };
  renderAt("/", { ...activities, items: [alreadyExcluded] });

  await userEvent.click(screen.getAllByRole("checkbox", { name: /Already excluded/ })[0]);

  for (const button of screen.getAllByRole("button", { name: /Exclude/ })) {
    expect(button).toBeDisabled();
  }
});
