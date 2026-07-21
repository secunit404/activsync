import { fireEvent, render, screen, within } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test } from "vitest";

import type { ActivitiesPage } from "@/lib/api";
import { ActivitiesView } from "./activities";

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

function renderAt(path: string) {
  const router = createMemoryRouter(
    [{ path: "*", element: <ActivitiesView data={activities} /> }],
    { initialEntries: [path] },
  );
  render(<RouterProvider router={router} />);
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
    screen.getByRole("tablist", { name: "Filter activities by status" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: /^All/ })).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: /Missing/ })).toBeInTheDocument();
});

// Ported from the deleted home.test.tsx ("reports activity filter changes
// and resets the page"). ActivityFilterPills now owns the URL directly
// instead of calling back into an onQueryChange prop, so this asserts the
// resulting URL state rather than a spy call.
test("selecting a filter resets the page to 1", () => {
  const router = renderAt("/?status=published&page=4");

  fireEvent.click(screen.getByRole("tab", { name: /Held/ }));

  const search = new URLSearchParams(router.state.location.search);
  expect(search.get("status")).toBe("held");
  expect(search.has("page")).toBe(false);
});

test("leaves a slot for the Task 8 activity table", () => {
  renderAt("/");
  expect(screen.getByTestId("activities-table-slot")).toBeInTheDocument();
});
