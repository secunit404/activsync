import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, Link, Outlet, RouterProvider } from "react-router";
import { expect, test, vi } from "vitest";

import { ACTIVITIES_HEADING_ID } from "@/lib/activity-focus";
import type { Activity, AppState } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import ActivityDetail from "./activity-detail";

const { getAppState } = vi.hoisted(() => ({ getAppState: vi.fn() }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, getAppState };
});

function activity(id: number, title: string): Activity {
  return {
    garminActivityId: id,
    activityType: "running",
    title,
    description: "",
    startTime: "2026-07-21 06:42:00",
    publishStatus: "pending",
    stravaActivityId: null,
    holdReason: null,
    startDateDisplay: "21 Jul",
    startMonthYearDisplay: "July 2026",
    startClockDisplay: "06:42",
    garminUrl: "https://connect.garmin.com/modern/activity/1",
    stravaUrl: null,
    hevyBadge: null,
    detail: {} as Activity["detail"],
  };
}

const first = activity(1, "First run");
const second = activity(2, "Second run");
const third = activity(3, "Third run");

/**
 * `listed` is what the list behind the overlay still shows when it closes;
 * the overlay itself was opened while all three were in the filter.
 */
function renderList(opened: Activity, listed: Activity[]) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  client.setQueryData(queryKeys.appState, {
    connections: { broken: [] },
    pollIntervals: { garminMinutes: 20, stravaMinutes: 5 },
  } as unknown as AppState);

  function ListPage() {
    return (
      <>
        <h1 id={ACTIVITIES_HEADING_ID} tabIndex={-1}>
          Activities
        </h1>
        {listed.map((item) => (
          <Link
            key={item.garminActivityId}
            to={`/${item.garminActivityId}`}
            data-activity-link={item.garminActivityId}
          >
            {item.title}
          </Link>
        ))}
        <Outlet context={[first, second, third]} />
      </>
    );
  }

  const router = createMemoryRouter(
    [{ path: "/", element: <ListPage />, children: [{ path: ":id", element: <ActivityDetail /> }] }],
    { initialEntries: [`/${opened.garminActivityId}`] },
  );
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return router;
}

test("closing the detail returns focus to the row that opened it", async () => {
  renderList(second, [first, second, third]);
  expect(screen.getByRole("dialog")).toBeInTheDocument();

  await userEvent.keyboard("{Escape}");

  await waitFor(() => expect(screen.getByRole("link", { name: "Second run" })).toHaveFocus());
});

test("the close button returns focus the same way", async () => {
  renderList(second, [first, second, third]);

  await userEvent.click(screen.getByRole("button", { name: "Close" }));

  await waitFor(() => expect(screen.getByRole("link", { name: "Second run" })).toHaveFocus());
});

test("navigating Back out of the detail returns focus to the row", async () => {
  const router = renderList(second, [first, second, third]);

  await router.navigate("/");

  await waitFor(() => expect(screen.getByRole("link", { name: "Second run" })).toHaveFocus());
});

test("a row that left the filter hands focus to the next row", async () => {
  renderList(second, [first, third]);

  await userEvent.keyboard("{Escape}");

  await waitFor(() => expect(screen.getByRole("link", { name: "Third run" })).toHaveFocus());
});

test("the last row falls back to the one before it", async () => {
  renderList(third, [first, second]);

  await userEvent.keyboard("{Escape}");

  await waitFor(() => expect(screen.getByRole("link", { name: "Second run" })).toHaveFocus());
});

test("an emptied list falls back to the page heading", async () => {
  renderList(second, []);

  await userEvent.keyboard("{Escape}");

  await waitFor(() => expect(screen.getByRole("heading", { name: "Activities" })).toHaveFocus());
});
