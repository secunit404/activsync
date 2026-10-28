import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect, test } from "vitest";

import type { ActivityView } from "@/lib/activity-view";
import type { PublishStatus } from "@/lib/api";
import { ActivityStatusFilter } from "./activity-status-filter";

const counts: Record<PublishStatus, number> = {
  pending: 2,
  held: 4,
  published: 34,
  missing: 3,
  excluded: 1,
};

function renderAt(path: string, value: ActivityView = "all", countsOverride = counts) {
  const router = createMemoryRouter(
    [{ path: "*", element: <ActivityStatusFilter counts={countsOverride} value={value} /> }],
    { initialEntries: [path] },
  );
  render(<RouterProvider router={router} />);
  return router;
}

test("is one radio group whose options carry their counts", () => {
  renderAt("/");

  expect(screen.getByRole("radiogroup", { name: "Show activities" })).toBeInTheDocument();
  expect(screen.getAllByRole("radio").map((radio) => radio.textContent)).toEqual([
    "Needs review7",
    "Pending2",
    "Published34",
    "Excluded1",
    "All44",
  ]);
});

test("Needs review counts held and missing, and the parts add up to All", () => {
  renderAt("/");

  const [review, pending, published, excluded, all] = screen
    .getAllByRole("radio")
    .map((radio) => Number(radio.textContent?.replace(/\D/g, "")));
  expect(review).toBe(counts.held + counts.missing);
  expect(review + pending + published + excluded).toBe(all);
});

test("marks the value on screen as checked and only it is tabbable", () => {
  renderAt("/?status=review", "review");

  const review = screen.getByRole("radio", { name: /Needs review/ });
  expect(review).toBeChecked();
  expect(review).toHaveAttribute("tabindex", "0");
  expect(screen.getByRole("radio", { name: /^All/ })).not.toBeChecked();
  expect(screen.getByRole("radio", { name: /^All/ })).toHaveAttribute("tabindex", "-1");
});

test("selecting writes the status param and resets the page", () => {
  const router = renderAt("/?status=review&page=4&sort=oldest", "review");

  fireEvent.click(screen.getByRole("radio", { name: /Pending/ }));

  const search = new URLSearchParams(router.state.location.search);
  expect(search.get("status")).toBe("pending");
  expect(search.get("sort")).toBe("oldest");
  expect(search.has("page")).toBe(false);
});

test("selecting All writes an explicit status=all", () => {
  const router = renderAt("/?status=review", "review");

  fireEvent.click(screen.getByRole("radio", { name: /^All/ }));

  expect(new URLSearchParams(router.state.location.search).get("status")).toBe("all");
});

test("excluded activities are one tap away", () => {
  const router = renderAt("/");

  fireEvent.click(screen.getByRole("radio", { name: /Excluded/ }));

  expect(new URLSearchParams(router.state.location.search).get("status")).toBe("excluded");
});

test("arrow keys move focus and selection, wrapping at the ends", async () => {
  const router = renderAt("/?status=review", "review");
  screen.getByRole("radio", { name: /Needs review/ }).focus();

  await userEvent.keyboard("{ArrowRight}");
  expect(screen.getByRole("radio", { name: /Pending/ })).toHaveFocus();
  expect(new URLSearchParams(router.state.location.search).get("status")).toBe("pending");

  screen.getByRole("radio", { name: /Needs review/ }).focus();
  await userEvent.keyboard("{ArrowLeft}");
  expect(screen.getByRole("radio", { name: /^All/ })).toHaveFocus();
  expect(new URLSearchParams(router.state.location.search).get("status")).toBe("all");
});

test("a legacy single-status link checks nothing but keeps the group reachable", () => {
  renderAt("/?status=held", "held");

  for (const radio of screen.getAllByRole("radio")) {
    expect(radio).not.toBeChecked();
  }
  expect(screen.getByRole("radio", { name: /Needs review/ })).toHaveAttribute("tabindex", "0");
});

test("the review count is the only coloured count, and only while something waits", () => {
  renderAt("/");
  expect(screen.getByText("7")).toHaveClass("text-warning");
  expect(screen.getByText("2")).not.toHaveClass("text-warning");
});

test("an empty inbox count stays quiet", () => {
  renderAt("/", "all", { ...counts, held: 0, missing: 0 });
  expect(screen.getByText("0", { selector: "span" })).toHaveClass("text-muted-foreground");
});
