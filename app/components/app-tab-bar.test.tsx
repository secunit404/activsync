import { render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test } from "vitest";

import { AppTabBar } from "./app-tab-bar";

function renderTabBar(hidden?: boolean, path = "/") {
  const router = createMemoryRouter(
    [
      { path: "/", element: <AppTabBar hidden={hidden} /> },
      { path: "/hevy", element: <AppTabBar hidden={hidden} /> },
      { path: "/hevy/mappings", element: <AppTabBar hidden={hidden} /> },
    ],
    { initialEntries: [path] },
  );
  render(<RouterProvider router={router} />);
}

test("is visible by default", () => {
  renderTabBar();
  expect(screen.getByTestId("app-tab-bar")).toBeVisible();
});

// The bulk-action bar occupies this exact slot on mobile while rows are
// selected — the two must never both be visible, so `hidden` needs to
// actually remove this from the a11y tree/layout, not just look inert.
test("hides when the bulk-action bar is occupying its slot", () => {
  renderTabBar(true);
  const nav = screen.getByTestId("app-tab-bar");
  expect(nav).not.toBeVisible();
  expect(nav).toHaveClass("hidden");
  expect(nav).not.toHaveClass("block");
});

test("renders its three nav links when visible", () => {
  renderTabBar(false);
  expect(screen.getByRole("link", { name: "Activities" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Hevy" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Settings" })).toBeInTheDocument();
});

test("marks the active destination, including nested routes", () => {
  renderTabBar(false, "/hevy/mappings");
  expect(screen.getByRole("link", { name: "Hevy" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("link", { name: "Activities" })).not.toHaveAttribute("aria-current");
});

test("slides the indicator under the active tab", () => {
  renderTabBar(false, "/hevy/mappings");
  expect(screen.getByTestId("app-tab-bar-indicator")).toHaveStyle({
    transform: "translateX(100%)",
  });
});
