import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import { PUBLISH_UNDO_WINDOW_MS } from "@/hooks/use-publish-queue";
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

// Counts are kept distinct per status so a count in an accessible name
// identifies exactly one filter option.
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
  appState?: Pick<AppState, "connections" | "catchUpReport"> &
    Partial<Pick<AppState, "pollIntervals">>,
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
    screen.getByText("Decide what reaches Strava, and see everything synced from Garmin."),
  ).toBeInTheDocument();
});

test("shows the status counts as one filter control, not as separate tiles", () => {
  renderAt("/");
  expect(screen.getByRole("radiogroup", { name: "Show activities" })).toBeInTheDocument();
  expect(screen.getByRole("radio", { name: /Needs review\s*8/ })).toBeInTheDocument();
  expect(screen.getByRole("radio", { name: /Excluded\s*1/ })).toBeInTheDocument();
  expect(screen.getByRole("radio", { name: /^All\s*45/ })).toBeInTheDocument();
  expect(screen.queryByText("THIS WEEK")).not.toBeInTheDocument();
});

test("with no status param the control reflects the list on screen", () => {
  renderAt("/", { ...activities, status: "review" });
  expect(screen.getByRole("radio", { name: /Needs review/ })).toBeChecked();
});

// Ported from the deleted home.test.tsx ("reports activity filter changes
// and resets the page"). The filter control owns the URL directly instead of
// calling back into an onQueryChange prop, so this asserts the resulting URL
// state rather than a spy call.
test("selecting a filter resets the page to 1", () => {
  const router = renderAt("/?status=published&page=4");

  fireEvent.click(screen.getByRole("radio", { name: /Needs review/ }));

  const search = new URLSearchParams(router.state.location.search);
  expect(search.get("status")).toBe("review");
  expect(search.has("page")).toBe(false);
});

test("a cleared inbox says so and offers every activity in one tap", () => {
  renderAt("/?status=review&sort=oldest", { ...activities, status: "review", items: [] });
  expect(screen.getByText("Nothing waiting for you")).toBeInTheDocument();
  expect(screen.queryByText("No activities found")).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Show all activities" })).toHaveAttribute(
    "href",
    "/?status=all&sort=oldest",
  );
});

const connected: Pick<AppState, "connections" | "catchUpReport" | "pollIntervals"> = {
  connections: {
    garmin: { connected: true, status: "", meta: "", email: "" },
    strava: { connected: true, status: "", meta: "" },
    broken: [],
  },
  catchUpReport: null,
  pollIntervals: { garminMinutes: 20, stravaMinutes: 5 },
};

test("an empty history says when Garmin is checked next and points at the connections", () => {
  renderAt("/", activities, connected);
  expect(screen.getByText("No activities yet")).toBeInTheDocument();
  expect(
    screen.getByText(
      "ActivSync checks Garmin every 20 min. New activities show up here after the next check.",
    ),
  ).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Check connections" })).toHaveAttribute(
    "href",
    "/settings#connections",
  );
});

test.each([
  ["pending", "Nothing waiting to publish", "Choose which types auto-sync"],
  ["held", "Nothing held for review", "Show all activities"],
  ["missing", "Nothing missing from Strava", "Show all activities"],
  ["published", "Nothing published yet", "Show what needs review"],
  ["excluded", "Nothing excluded", "Show what needs review"],
] as const)("an empty %s filter explains itself", (status, title, nextStep) => {
  renderAt(`/?status=${status}`, { ...activities, status, items: [] });
  const empty = screen.getByText(title).closest<HTMLElement>("[data-slot='empty']")!;
  if (nextStep) {
    expect(within(empty).getByRole("link", { name: nextStep })).toBeInTheDocument();
  } else {
    expect(within(empty).queryByRole("link")).not.toBeInTheDocument();
  }
});

test.each([
  ["published", "Nothing published yet"],
  ["excluded", "Nothing excluded"],
] as const)("an empty %s filter offers review only while something waits", (status, title) => {
  renderAt(`/?status=${status}`, {
    ...activities,
    status,
    counts: { ...activities.counts, held: 0, missing: 0 },
  });
  expect(screen.getByText(title)).toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Show what needs review" })).not.toBeInTheDocument();
});

test("the selected filter is explained in one line, with the Strava check interval for pending", async () => {
  const router = renderAt("/?status=review", { ...activities, status: "review" }, connected);
  const group = screen.getByRole("radiogroup", { name: "Show activities" });
  expect(group).toHaveAccessibleDescription("Held or removed from Strava. These wait for you.");

  await act(() => router.navigate("/?status=pending"));
  expect(group).toHaveAccessibleDescription("Publishes to Strava on its own, usually within 5 min.");

  await act(() => router.navigate("/?status=excluded"));
  expect(group).toHaveAccessibleDescription("Kept off Strava for good, unless you restore it.");
});

test("pending and its empty state promise nothing while Strava is disconnected", () => {
  renderAt("/?status=pending", { ...activities, status: "pending", items: [] }, {
    ...connected,
    connections: { ...connected.connections, broken: ["strava"] },
  });
  expect(screen.getByRole("radiogroup", { name: "Show activities" })).toHaveAccessibleDescription(
    "Publishes to Strava once Strava is reconnected.",
  );
  expect(
    screen.getByText(
      "New activities whose type has auto-sync on wait here until Strava is reconnected.",
    ),
  ).toBeInTheDocument();
});

test("backlog and missing rows say why they wait, on the table and the cards", () => {
  renderAt("/?status=review", {
    ...activities,
    status: "review",
    items: [
      { ...oneActivity, publishStatus: "held", holdReason: "backlog" },
      { ...anotherActivity, publishStatus: "missing" },
      { ...oneActivity, garminActivityId: 7, title: "Ride", publishStatus: "held" },
    ],
  });
  const table = screen.getByTestId("activities-table");
  const cards = screen.getByTestId("activities-cards");
  for (const scope of [table, cards]) {
    expect(within(scope).getByText("Older activity found after a reconnect")).toBeInTheDocument();
    expect(within(scope).getByText("Removed from Strava after publishing")).toHaveAttribute(
      "title",
      "Removed from Strava after publishing",
    );
    expect(within(scope).queryByText("Auto-sync is off for this type")).not.toBeInTheDocument();
  }
});

test("pending rows carry no reason line", () => {
  renderAt("/", { ...activities, items: [oneActivity] });
  expect(screen.queryByText("Auto-sync is off for this type")).not.toBeInTheDocument();
  expect(screen.queryByText("Removed from Strava after publishing")).not.toBeInTheDocument();
});

test("merges both broken connections into one banner naming each service", () => {
  renderAt("/", activities, {
    connections: {
      garmin: { connected: false, status: "", meta: "", email: "" },
      strava: { connected: false, status: "", meta: "" },
      broken: ["garmin", "strava"],
    },
    catchUpReport: null,
  });

  expect(screen.getByText("Reconnect Garmin and Strava")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Reconnect Garmin" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Reconnect Strava" })).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
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
  expect(screen.queryByRole("button", { name: /reconnect/i })).not.toBeInTheDocument();
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
  // both subtrees in the DOM regardless of viewport.
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

test("the selection count is announced through a polite live region", async () => {
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] });
  const status = screen
    .getAllByRole("status")
    .find((node) => node.classList.contains("sr-only"));
  expect(status).toBeDefined();
  expect(status).toHaveTextContent("");

  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);

  expect(status).toHaveTextContent("1 selected");
});

// Phones show cards, with no table header to hold a select-all, so the
// dock carries one while a selection exists. jsdom applies no CSS, so both
// it and the (desktop) table header are in the tree here.
test("the dock's select-all goes from mixed to the whole page, then to none", async () => {
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] });
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);

  const bar = screen.getByTestId("bulk-action-bar");
  const selectAll = within(bar).getByRole("checkbox", { name: "Select all activities" });
  expect(selectAll).toHaveAttribute("aria-checked", "mixed");

  await userEvent.click(selectAll);

  expect(within(bar).getAllByText("2 selected").length).toBeGreaterThan(0);
  expect(selectAll).toHaveAttribute("aria-checked", "true");

  await userEvent.click(selectAll);

  expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument();
});

test("Escape clears the selection", async () => {
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] });
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);
  expect(screen.getByTestId("bulk-action-bar")).toBeInTheDocument();

  await userEvent.keyboard("{Escape}");

  expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument();
});

// The detail overlay renders over the list with the selection alive behind
// it; Escape there closes the overlay and must leave the selection alone.
test("Escape inside an open dialog leaves the selection alone", async () => {
  renderAt("/", { ...activities, items: [oneActivity, anotherActivity] });
  await userEvent.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);
  const dialog = document.createElement("div");
  dialog.setAttribute("role", "dialog");
  document.body.append(dialog);

  try {
    await userEvent.keyboard("{Escape}");
    expect(screen.getByTestId("bulk-action-bar")).toBeInTheDocument();
  } finally {
    dialog.remove();
  }
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
// AttentionBanner's copy: "Synced activities still publish").
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

  fireEvent.click(screen.getByRole("radio", { name: /Needs review/ }));

  expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument();
});

test("publishing the selection clears it, marks the rows publishing, and sends after the undo window", async () => {
  publishActivities.mockResolvedValue({
    message: "Published 1 activity to Strava",
    severity: "success",
    publishedCount: 1,
    failedCount: 0,
    blockedCount: 0,
  });
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    renderAt("/", { ...activities, items: [oneActivity, anotherActivity] });
    await user.click(screen.getAllByRole("checkbox", { name: /Morning run/ })[0]);

    await user.click(screen.getAllByRole("button", { name: /Publish 1/ })[0]);

    expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument();
    expect(publishActivities).not.toHaveBeenCalled();
    const morningRow = within(screen.getByRole("table")).getByRole("row", { name: /Morning run/ });
    expect(within(morningRow).getByText("PUBLISHING…")).toBeInTheDocument();
    const pushDayRow = within(screen.getByRole("table")).getByRole("row", { name: /Push Day/ });
    expect(within(pushDayRow).queryByText("PUBLISHING…")).not.toBeInTheDocument();

    await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS));

    expect(publishActivities).toHaveBeenCalledWith([12345]);
    await waitFor(() => expect(within(morningRow).queryByText("PUBLISHING…")).not.toBeInTheDocument());
  } finally {
    vi.useRealTimers();
  }
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
