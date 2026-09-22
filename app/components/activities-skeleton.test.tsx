import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { ActivitiesSkeleton } from "./activities-skeleton";

test("announces itself as a loading status", () => {
  render(<ActivitiesSkeleton />);
  expect(screen.getByRole("status", { name: "Loading activities" })).toBeInTheDocument();
});

// Both the desktop table skeleton and the mobile card skeleton are present
// in the DOM (switched by CSS breakpoint only, same convention as
// ActivitiesTable/ActivityCard — see activities.test.tsx's docstring on the
// stat strip for why jsdom always renders both subtrees).
test("mirrors the real layout's dual desktop/mobile list shape", () => {
  render(<ActivitiesSkeleton />);
  expect(screen.getByTestId("activities-skeleton-rows").children.length).toBeGreaterThan(0);
  expect(screen.getByTestId("activities-skeleton-cards").children.length).toBeGreaterThan(0);
});
