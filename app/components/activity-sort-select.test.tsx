import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test } from "vitest";

import { ActivitySortSelect } from "./activity-sort-select";

function renderSelect(initialEntry = "/") {
  const router = createMemoryRouter(
    [{ path: "/", element: <ActivitySortSelect /> }],
    { initialEntries: [initialEntry] },
  );
  render(<RouterProvider router={router} />);
  return router;
}

test("defaults to newest when the URL carries no sort", () => {
  renderSelect();
  expect(screen.getByLabelText("Sort activities")).toHaveValue("newest");
});

test("reflects an explicit sort from the URL", () => {
  renderSelect("/?sort=oldest");
  expect(screen.getByLabelText("Sort activities")).toHaveValue("oldest");
});

test("an unrecognised sort falls back to newest", () => {
  renderSelect("/?sort=sideways");
  expect(screen.getByLabelText("Sort activities")).toHaveValue("newest");
});

test("choosing oldest writes the param and drops the page", async () => {
  const router = renderSelect("/?page=3&status=held");

  await userEvent.selectOptions(screen.getByLabelText("Sort activities"), "oldest");

  const params = new URLSearchParams(router.state.location.search);
  expect(params.get("sort")).toBe("oldest");
  // A re-sorted list makes page 3 meaningless — same rule the filter pills
  // already apply when the status changes.
  expect(params.get("page")).toBeNull();
  // Unrelated params survive.
  expect(params.get("status")).toBe("held");
});

test("returning to newest clears the param rather than writing the default", async () => {
  const router = renderSelect("/?sort=oldest");

  await userEvent.selectOptions(screen.getByLabelText("Sort activities"), "newest");

  expect(new URLSearchParams(router.state.location.search).get("sort")).toBeNull();
});
