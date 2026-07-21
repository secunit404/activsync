import { fireEvent, render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test } from "vitest";

import { ActivityFilterPills } from "./activity-filter-pills";
import type { PublishStatus } from "@/lib/api";

const counts: Record<PublishStatus, number> = {
  pending: 2,
  held: 2,
  published: 34,
  missing: 3,
  excluded: 1,
};

function renderAt(path: string) {
  const router = createMemoryRouter(
    [{ path: "*", element: <ActivityFilterPills counts={counts} /> }],
    { initialEntries: [path] },
  );
  render(<RouterProvider router={router} />);
  return router;
}

test("marks the status matching the URL as selected", () => {
  renderAt("/?status=held");
  expect(screen.getByRole("button", { name: /Held/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(screen.getByRole("button", { name: /^All/ })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
});

test("defaults to All when there is no status param", () => {
  renderAt("/");
  expect(screen.getByRole("button", { name: /^All/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("does not render an Excluded pill", () => {
  renderAt("/");
  expect(screen.queryByRole("button", { name: /Excluded/ })).not.toBeInTheDocument();
});

test("selecting a filter writes the status param and resets the page", () => {
  const router = renderAt("/?status=published&page=4");

  fireEvent.click(screen.getByRole("button", { name: /Pending/ }));

  const search = new URLSearchParams(router.state.location.search);
  expect(search.get("status")).toBe("pending");
  expect(search.has("page")).toBe(false);
});

test("selecting All removes the status param", () => {
  const router = renderAt("/?status=held");

  fireEvent.click(screen.getByRole("button", { name: /^All/ }));

  const search = new URLSearchParams(router.state.location.search);
  expect(search.has("status")).toBe(false);
});
