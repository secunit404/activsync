import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, Outlet, RouterProvider } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import { PUBLISH_UNDO_WINDOW_MS } from "@/hooks/use-publish-queue";
import type { Activity, AppState } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import ActivityDetail from "./activity-detail";

const { editActivity, publishActivity, excludeActivity, restoreActivity, getAppState } =
  vi.hoisted(() => ({
    editActivity: vi.fn(),
    publishActivity: vi.fn(),
    excludeActivity: vi.fn(),
    restoreActivity: vi.fn(),
    getAppState: vi.fn(),
  }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, editActivity, publishActivity, excludeActivity, restoreActivity, getAppState };
});

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

const fixture: Activity = {
  garminActivityId: 12345,
  activityType: "running",
  title: "Morning run",
  description: "Easy shakeout along the river loop.",
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
  detail: { ...baseDetail, distance: "10.2 km", duration: "52:14", avgHr: "156 bpm" },
};

const appStateFixture: AppState = {
  name: "ActivSync",
  version: "0.1.0",
  release: true,
  development: true,
  setup: { complete: true, step: null },
  connections: {
    garmin: { connected: true, status: "Connected", meta: "", email: "athlete@example.com" },
    strava: { connected: true, status: "Connected", meta: "" },
    broken: [],
  },
  hevy: { enabled: false, connected: false, status: "" },
  catchUpReport: null,
  update: { latest: null, available: false, repoUrl: "", releaseUrl: "" },
  reviewCount: 0,
  pollIntervals: { garminMinutes: 20, stravaMinutes: 5 },
};

getAppState.mockResolvedValue(appStateFixture);

beforeEach(() => {
  publishActivity.mockClear();
});

function renderDetail(
  activity: Activity | null,
  {
    edit = false,
    activities = activity ? [activity] : [],
    broken = [] as AppState["connections"]["broken"],
    listSearch = "",
  }: {
    edit?: boolean;
    activities?: Activity[];
    broken?: AppState["connections"]["broken"];
    listSearch?: string;
  } = {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  client.setQueryData(queryKeys.appState, {
    ...appStateFixture,
    connections: { ...appStateFixture.connections, broken },
  });

  const id = activity?.garminActivityId ?? 999;
  const search = new URLSearchParams(listSearch);
  if (edit) {
    search.set("edit", "1");
  }
  const initialPath = `/${id}${search.size > 0 ? `?${search}` : ""}`;

  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: <Outlet context={activities} />,
        children: [
          { index: true, element: <div>List</div> },
          { path: ":id", element: <ActivityDetail /> },
        ],
      },
    ],
    { initialEntries: [initialPath] },
  );

  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );

  return { router, client };
}

test.each([
  [{ publishStatus: "held", holdReason: "category" }, "HELD", "Auto-sync is off for this type"],
  [{ publishStatus: "held", holdReason: "backlog" }, "HELD", "Older activity found after a reconnect"],
  [
    { publishStatus: "missing", holdReason: null },
    "MISSING",
    "Removed from Strava after publishing. Republish or exclude.",
  ],
  [
    { publishStatus: "pending", holdReason: null },
    "PENDING",
    "Publishes to Strava on its own, usually within 5 min.",
  ],
] as const)("the header shows the status and what it means (%o)", (status, pill, line) => {
  renderDetail({ ...fixture, ...status });
  const header = screen.getByTestId("activity-status-line");
  expect(within(header).getByText(pill)).toBeInTheDocument();
  expect(header).toHaveTextContent(line);
  expect(screen.getByRole("dialog")).toHaveAccessibleDescription(expect.stringContaining(line));
});

test("unpublished activity warns that edits reach Garmin only", () => {
  renderDetail({ ...fixture, publishStatus: "pending" }, { edit: true });
  expect(screen.getByRole("status")).toHaveTextContent(/Garmin only/i);
});

test("published activity says edits reach both services", () => {
  renderDetail({ ...fixture, publishStatus: "published" }, { edit: true });
  expect(screen.getByRole("status")).toHaveTextContent(/Garmin and Strava/i);
});

test("closing the overlay returns to the list", async () => {
  const { router } = renderDetail(fixture, {});
  await userEvent.keyboard("{Escape}");
  expect(router.state.location.pathname).toBe("/");
});

test("closing the overlay returns to the same filtered list", async () => {
  const { router } = renderDetail(fixture, { listSearch: "status=all&page=2" });
  await userEvent.keyboard("{Escape}");
  expect(router.state.location.pathname).toBe("/");
  expect(router.state.location.search).toBe("?status=all&page=2");
});

test("entering edit mode keeps the list's filter in the URL", async () => {
  const { router } = renderDetail(fixture, { listSearch: "status=held" });
  await userEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);
  expect(router.state.location.search).toBe("?status=held&edit=1");
});

test("renders the stat grid, description and Garmin link in view mode", () => {
  renderDetail(fixture);
  expect(screen.getByText("10.2 km")).toBeInTheDocument();
  expect(screen.getByText("Easy shakeout along the river loop.")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "View on Garmin" })).toHaveAttribute(
    "href",
    fixture.garminUrl,
  );
});

test("preserves the description's saved line breaks", () => {
  const multilineDescription = [
    "🏋️ Afternoon workout 💪",
    "",
    "Chest Fly (Machine): 3 sets - 57.5kg × 9",
    "Lat Pulldown (Cable): 3 sets - 50.0kg × 9",
  ].join("\n");
  renderDetail({ ...fixture, description: multilineDescription });

  const description = screen.getByText("Description").nextElementSibling;
  expect(description?.textContent).toBe(multilineDescription);
  expect(description).toHaveClass("whitespace-pre-wrap", "break-words");
});

test("does not render a Strava link when stravaUrl is null", () => {
  renderDetail(fixture);
  expect(screen.queryByRole("link", { name: /Strava/ })).not.toBeInTheDocument();
});

test("renders a View on Strava link once the activity is published", () => {
  renderDetail({ ...fixture, publishStatus: "published", stravaUrl: "https://strava.com/activities/1" });
  const link = screen.getByRole("link", { name: "View on Strava" });
  expect(link).toHaveAttribute("href", "https://strava.com/activities/1");
  expect(link).toHaveClass("font-bold");
});

test("an id not present in the current list shows a real not-found message, not a blank overlay", () => {
  renderDetail(null, { activities: [] });
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  expect(screen.getByText(/isn't in the current list/i)).toBeInTheDocument();
});

test("Save stays disabled until a field changes, and re-disables when reverted", async () => {
  renderDetail(fixture, { edit: true });
  const saveButton = screen.getByRole("button", { name: "Save changes" });
  expect(saveButton).toBeDisabled();

  const titleInput = screen.getByLabelText("Title");
  await userEvent.type(titleInput, "!");
  expect(saveButton).not.toBeDisabled();

  await userEvent.type(titleInput, "{Backspace}");
  expect(saveButton).toBeDisabled();
});

test("saving calls editActivity with the edited fields and returns to view mode", async () => {
  editActivity.mockResolvedValue({
    message: "Saved changes to Morning run",
    severity: "success",
    publishedCount: 0,
    failedCount: 0,
    blockedCount: 0,
  });
  const { router } = renderDetail(fixture, { edit: true });

  await userEvent.type(screen.getByLabelText("Title"), "!");
  await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

  await waitFor(() => expect(editActivity).toHaveBeenCalledWith(12345, "Morning run!", fixture.description));
  await waitFor(() => expect(router.state.location.search).toBe(""));
});

test("clicking Edit switches to edit mode via the ?edit=1 search param", async () => {
  const { router } = renderDetail(fixture);
  await userEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);
  expect(router.state.location.search).toBe("?edit=1");
  expect(screen.getByLabelText("Title")).toBeInTheDocument();
});

test("shows Restore instead of Exclude for an excluded activity, and no Publish button", () => {
  renderDetail({ ...fixture, publishStatus: "excluded" });
  expect(screen.getAllByRole("button", { name: "Restore" }).length).toBeGreaterThan(0);
  expect(screen.queryByRole("button", { name: "Exclude" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Publish/ })).not.toBeInTheDocument();
});

test("a published activity only offers Edit, no Publish or Exclude", () => {
  renderDetail({ ...fixture, publishStatus: "published" });
  expect(screen.getAllByRole("button", { name: "Edit" }).length).toBeGreaterThan(0);
  expect(screen.queryByRole("button", { name: "Exclude" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Publish/ })).not.toBeInTheDocument();
});

test("disables Publish when Strava is disconnected", () => {
  renderDetail(fixture, { broken: ["strava"] });
  for (const button of screen.getAllByRole("button", { name: /Publish/ })) {
    expect(button).toBeDisabled();
  }
});

test("publishing waits out the undo window, then calls publishActivity with the activity id", async () => {
  publishActivity.mockResolvedValue({
    message: "Published Morning run to Strava",
    severity: "success",
    publishedCount: 1,
    failedCount: 0,
    blockedCount: 0,
  });
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    renderDetail(fixture);
    await user.click(screen.getAllByRole("button", { name: /Publish/ })[0]);

    expect(screen.getAllByRole("button", { name: "Undo publish" }).length).toBeGreaterThan(0);
    expect(screen.getByRole("status")).toHaveTextContent(/Publishing to Strava in 5 seconds/);
    for (const button of screen.getAllByRole("button", { name: "Exclude" })) {
      expect(button).toBeDisabled();
    }
    expect(publishActivity).not.toHaveBeenCalled();

    await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS));

    expect(publishActivity).toHaveBeenCalledWith(12345);
  } finally {
    vi.useRealTimers();
  }
});

test("Undo publish in the overlay cancels before anything is sent", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    renderDetail(fixture);
    await user.click(screen.getAllByRole("button", { name: /Publish/ })[0]);
    await user.click(screen.getAllByRole("button", { name: "Undo publish" })[0]);

    await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS * 2));

    expect(publishActivity).not.toHaveBeenCalled();
    expect(screen.getAllByRole("button", { name: "Publish to Strava" }).length).toBeGreaterThan(0);
  } finally {
    vi.useRealTimers();
  }
});
