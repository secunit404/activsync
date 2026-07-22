import { render, screen } from "@testing-library/react";
import { createRoutesStub } from "react-router";
import { expect, test } from "vitest";

import { MappingListRow, type MappingRowData } from "./mapping-list-row";

function mapping(overrides: Partial<MappingRowData> = {}): MappingRowData {
  return {
    templateId: "tpl-1",
    title: "Landmine Press",
    isCustom: true,
    muscleGroup: "shoulders",
    mapped: true,
    unmapped: false,
    garminRejected: false,
    suggested: false,
    category: 3,
    subcategory: 7,
    categoryName: "Strength",
    subcategoryName: "Shoulder Press",
    ...overrides,
  };
}

function renderRow(value: MappingRowData) {
  const Stub = createRoutesStub([
    {
      path: "/hevy/mappings",
      Component: () => (
        <ul>
          <MappingListRow mapping={value} />
        </ul>
      ),
    },
  ]);
  render(<Stub initialEntries={["/hevy/mappings"]} />);
}

test("a mapped row shows where it syncs and offers Edit", () => {
  renderRow(mapping());
  expect(screen.getByText("Strength › Shoulder Press")).toBeVisible();
  // Relative to the list page, so the editor opens as its child.
  expect(screen.getByRole("link", { name: "Edit Landmine Press" })).toHaveAttribute(
    "href",
    "/hevy/mappings/mapping/tpl-1",
  );
});

test("an unmapped row says so and offers Map", () => {
  renderRow(
    mapping({
      mapped: false,
      unmapped: true,
      categoryName: null,
      subcategoryName: null,
    }),
  );
  expect(screen.getByText(/Unmapped/)).toBeVisible();
  expect(screen.getByRole("link", { name: "Map Landmine Press" })).toBeVisible();
});

test("a Garmin-rejected row offers Remap and says why", () => {
  renderRow(mapping({ garminRejected: true }));
  expect(screen.getByText("Rejected by Garmin")).toBeVisible();
  expect(screen.getByRole("link", { name: "Remap Landmine Press" })).toBeVisible();
});

test("a custom exercise is badged as such", () => {
  renderRow(mapping());
  expect(screen.getByText("CUSTOM")).toBeVisible();
});

test("a built-in exercise carries no custom badge", () => {
  renderRow(mapping({ isCustom: false }));
  expect(screen.queryByText("CUSTOM")).toBeNull();
});

// A mapped row whose taxonomy names came back empty must not render a bare
// "›" — it falls back to the plain status word.
test("a mapped row with no resolved names still reads sensibly", () => {
  renderRow(mapping({ categoryName: null, subcategoryName: null }));
  expect(screen.getByText("Mapped")).toBeVisible();
});
