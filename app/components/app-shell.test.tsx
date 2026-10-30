import { render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test } from "vitest";

import { AppRail } from "./app-rail";

function renderAt(path: string, reviewCount?: number) {
  const router = createMemoryRouter(
    [{ path: "*", element: <AppRail reviewCount={reviewCount} /> }],
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

test("rail badges Activities with the review count, capped at 99+", () => {
  renderAt("/hevy", 120);
  const link = screen.getByRole("link", { name: "Activities, 120 need review" });
  expect(link).toHaveTextContent("99+");
  expect(screen.getByRole("link", { name: "Hevy" })).not.toHaveTextContent(/\d/);
});

test("rail shows no badge when nothing needs review", () => {
  renderAt("/", 0);
  expect(screen.getByRole("link", { name: "Activities" })).toBeInTheDocument();
  expect(screen.queryByTestId("review-badge")).toBeNull();
});
