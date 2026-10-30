import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryRouter, Outlet, RouterProvider } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { ActivitiesPage, ActivityQuery, AppState } from "@/lib/api";
import Activities from "./activities";

const { getAppState, getActivities } = vi.hoisted(() => ({
  getAppState: vi.fn(),
  getActivities: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, getAppState, getActivities };
});

function appState(reviewCount: number): AppState {
  return {
    name: "ActivSync",
    version: "0.1.0",
    release: true,
    development: true,
    setup: { complete: true, step: null },
    connections: {
      garmin: { connected: true, status: "", meta: "", email: "" },
      strava: { connected: true, status: "", meta: "" },
      broken: [],
    },
    hevy: { enabled: false, connected: false, status: "" },
    catchUpReport: null,
    update: { latest: null, available: false, repoUrl: "", releaseUrl: "" },
    reviewCount,
  };
}

function pageFor(query: ActivityQuery): ActivitiesPage {
  return {
    items: [],
    sort: query.sort,
    status: query.status,
    counts: { pending: 1, held: 2, published: 3, missing: 0, excluded: 1 },
    weekTotal: { seconds: 0, display: "0m" },
    pagination: {
      page: 1,
      pageSize: 20,
      pageCount: 1,
      totalCount: 0,
      firstItem: 0,
      lastItem: 0,
    },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  getActivities.mockImplementation(async (query: ActivityQuery) => pageFor(query));
});

function renderAt(path: string) {
  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: <Outlet context={() => {}} />,
        children: [
          {
            path: "/",
            element: <Activities />,
            children: [{ path: ":id", element: <div>Detail</div> }],
          },
        ],
      },
    ],
    { initialEntries: [path] },
  );
  render(
    <QueryClientProvider client={new QueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return router;
}

function requestedStatuses() {
  return getActivities.mock.calls.map(([query]) => (query as ActivityQuery).status);
}

test("opens on Needs review when something is waiting for a decision", async () => {
  getAppState.mockResolvedValue(appState(2));
  const router = renderAt("/");

  await waitFor(() => expect(router.state.location.search).toBe("?status=review"));
  expect(await screen.findByRole("radio", { name: /Needs review/ })).toBeChecked();
  expect(requestedStatuses()).toEqual(["review"]);
});

test("opens on All when nothing needs review", async () => {
  getAppState.mockResolvedValue(appState(0));
  const router = renderAt("/");

  await waitFor(() => expect(router.state.location.search).toBe("?status=all"));
  expect(await screen.findByRole("radio", { name: /^All/ })).toBeChecked();
  expect(requestedStatuses()).toEqual([null]);
});

test("an explicit All is never replaced by the default", async () => {
  getAppState.mockResolvedValue(appState(2));
  const router = renderAt("/?status=all");

  expect(await screen.findByRole("radio", { name: /^All/ })).toBeChecked();
  expect(router.state.location.search).toBe("?status=all");
  expect(requestedStatuses()).toEqual([null]);
});

test("keeps sort when resolving the default", async () => {
  getAppState.mockResolvedValue(appState(2));
  const router = renderAt("/?sort=oldest");

  await waitFor(() =>
    expect(router.state.location.search).toBe("?sort=oldest&status=review"),
  );
});

test("a detail deep link without a status shows every activity behind it", async () => {
  getAppState.mockResolvedValue(appState(2));
  const router = renderAt("/123");

  expect(await screen.findByText("Detail")).toBeInTheDocument();
  expect(router.state.location.search).toBe("");
  expect(requestedStatuses()).toEqual([null]);
});

test("Back after choosing All returns to the inbox, not to a fresh default", async () => {
  getAppState.mockResolvedValue(appState(2));
  const router = renderAt("/");
  await waitFor(() => expect(router.state.location.search).toBe("?status=review"));

  await userEvent.click(await screen.findByRole("radio", { name: /^All/ }));
  await waitFor(() => expect(router.state.location.search).toBe("?status=all"));

  await router.navigate(-1);
  await waitFor(() => expect(router.state.location.search).toBe("?status=review"));
  await waitFor(() => expect(screen.getByRole("radio", { name: /Needs review/ })).toBeChecked());
});
