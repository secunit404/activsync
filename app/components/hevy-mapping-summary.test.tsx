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
    garminRejected: false,
    suggested: false,
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

test("shows a needs-mapping count and links each unmapped exercise to its editor", () => {
  renderSummary({ mappings: [mapping({})], categories: [] });

  expect(screen.getByText("1 need mapping")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /map bulgarian split squat/i })).toHaveAttribute(
    "href",
    "/hevy/mapping/tmpl-1",
  );
});

test("labels a Garmin-rejected mapping as Remap, not Map", () => {
  renderSummary({
    mappings: [mapping({ templateId: "tmpl-2", mapped: true, unmapped: false, garminRejected: true })],
    categories: [],
  });

  expect(screen.getByText(/rejected by garmin/i)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /remap/i })).toHaveAttribute(
    "href",
    "/hevy/mapping/tmpl-2",
  );
});

test("flags custom exercises inline", () => {
  renderSummary({ mappings: [mapping({ isCustom: true })], categories: [] });
  expect(screen.getByText("CUSTOM")).toBeInTheDocument();
});

test("omits rows that are already mapped and not rejected", () => {
  renderSummary({
    mappings: [mapping({ mapped: true, unmapped: false, garminRejected: false })],
    categories: [],
  });

  expect(screen.queryByText("Bulgarian Split Squat")).not.toBeInTheDocument();
  expect(screen.queryByText(/need mapping/i)).not.toBeInTheDocument();
});

test("shows a quiet all-mapped message and no badge when nothing needs attention", () => {
  renderSummary({ mappings: [], categories: [] });
  expect(screen.getByText(/every hevy exercise/i)).toBeInTheDocument();
  expect(screen.queryByText(/need mapping/i)).not.toBeInTheDocument();
});
