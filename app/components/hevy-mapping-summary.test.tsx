import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { expect, test } from "vitest";

import type { HevyToolsState } from "@/lib/api";
import { HevyMappingSummary } from "./hevy-mapping-summary";

type Mapping = HevyToolsState["mappings"][number];

function mapping(overrides: Partial<Mapping>): Mapping {
  return {
    templateId: "tmpl-1",
    title: "Bulgarian Split Squat",
    isCustom: false,
    muscleGroup: "Legs",
    mapped: false,
    unmapped: true,
    suggested: false,
    hasStandardMapping: false,
    standardCategory: null,
    standardSubcategory: null,
    source: "",
    category: null,
    subcategory: null,
    categoryName: null,
    subcategoryName: null,
    ...overrides,
  };
}

function renderSummary(tools: HevyToolsState) {
  return render(
    <MemoryRouter>
      <HevyMappingSummary tools={tools} />
    </MemoryRouter>,
  );
}

test("links each unmapped exercise to its editor", () => {
  renderSummary({ mappings: [mapping({})], categories: [] });

  // No count badge in the header: the rows below it are the count, and the
  // card only lists exercises that need mapping in the first place.
  expect(screen.queryByText(/need mapping/i)).not.toBeInTheDocument();
  const map = screen.getByRole("link", { name: /map bulgarian split squat/i });
  expect(map).toHaveAttribute("href", "/hevy/mapping/tmpl-1");
  expect(map).toHaveAttribute("data-variant", "warning");
});

test("flags custom exercises inline", () => {
  renderSummary({ mappings: [mapping({ isCustom: true })], categories: [] });
  expect(screen.getByText("CUSTOM")).toBeInTheDocument();
});

test("renders heuristic choices like every other unmapped row", () => {
  renderSummary({
    mappings: [
      mapping({
        suggested: true,
        categoryName: "Squat",
        subcategoryName: "Back squat",
      }),
    ],
    categories: [],
  });

  expect(screen.getByText("Unmapped")).toBeVisible();
  expect(screen.queryByText("Legs")).toBeNull();
  expect(screen.queryByText("Suggestion ready")).toBeNull();
  expect(screen.queryByText(/Workouts match Garmin activities by time/)).toBeNull();
  const map = screen.getByRole("link", { name: /map bulgarian split squat/i });
  expect(map).toBeVisible();
  expect(map).toHaveAttribute("data-variant", "warning");
});

test("omits rows that are already mapped and not rejected", () => {
  renderSummary({
    mappings: [mapping({ mapped: true, unmapped: false })],
    categories: [],
  });

  expect(screen.queryByText("Bulgarian Split Squat")).not.toBeInTheDocument();
  expect(screen.queryByText(/need mapping/i)).not.toBeInTheDocument();
});

test("shows a quiet all-mapped message when nothing needs attention", () => {
  renderSummary({ mappings: [], categories: [] });
  expect(screen.getByText(/every hevy exercise/i)).toBeInTheDocument();
  expect(screen.queryByText(/need mapping/i)).not.toBeInTheDocument();
});
