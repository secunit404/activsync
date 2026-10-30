import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test } from "vitest";

import { ActivitiesPagination } from "./activities-pagination";

function renderAt(path: string, props: { page: number; pageCount: number }) {
  const router = createMemoryRouter(
    [{ path: "*", element: <ActivitiesPagination {...props} /> }],
    { initialEntries: [path] },
  );
  render(<RouterProvider router={router} />);
  return router;
}

test("renders nothing when there is only one page", () => {
  renderAt("/", { page: 1, pageCount: 1 });
  expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
});

test("shows the current page and total page count", () => {
  renderAt("/", { page: 1, pageCount: 3 });
  expect(screen.getByText("Page 1 of 3")).toBeInTheDocument();
});

// Kept in place, disabled, so the cluster does not shift between pages.
test("disables Previous on the first page without removing it", () => {
  renderAt("/", { page: 1, pageCount: 3 });
  const previous = screen.getByRole("link", { name: "Go to previous page" });
  expect(previous).toHaveAttribute("aria-disabled", "true");
  expect(previous).not.toHaveAttribute("href");
  expect(screen.getByRole("link", { name: "Go to next page" })).toBeInTheDocument();
});

test("disables Next on the last page without removing it", () => {
  renderAt("/", { page: 3, pageCount: 3 });
  expect(screen.getByRole("link", { name: "Go to previous page" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Go to next page" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

// Ported from the deleted home.test.tsx ("reports next-page navigation"),
// updated for the URL-param convention ActivityFilterPills established:
// pagination now owns the `page` search param directly instead of calling
// back into an onQueryChange prop.
test("reports next-page navigation via the page URL param", async () => {
  const router = renderAt("/?status=held", { page: 1, pageCount: 2 });

  await userEvent.click(screen.getByRole("link", { name: "Go to next page" }));

  const search = new URLSearchParams(router.state.location.search);
  expect(search.get("page")).toBe("2");
  expect(search.get("status")).toBe("held");
});

test("going to the previous page from page 2 removes the page param", async () => {
  const router = renderAt("/", { page: 2, pageCount: 3 });

  await userEvent.click(screen.getByRole("link", { name: "Go to previous page" }));

  const search = new URLSearchParams(router.state.location.search);
  expect(search.has("page")).toBe(false);
});
