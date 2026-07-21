import { render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test } from "vitest";

import { AppRail } from "./app-rail";

function renderAt(path: string) {
  const router = createMemoryRouter(
    [{ path: "*", element: <AppRail /> }],
    { initialEntries: [path] },
  );
  return render(<RouterProvider router={router} />);
}

test("rail marks the active destination", () => {
  renderAt("/hevy");
  expect(screen.getByRole("link", { name: "Hevy" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("link", { name: "Activities" })).not.toHaveAttribute("aria-current");
});

test("rail exposes all three destinations", () => {
  renderAt("/");
  expect(screen.getAllByRole("link")).toHaveLength(3);
});
