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
    suggested: false,
    hasStandardMapping: false,
    standardCategory: null,
    standardSubcategory: null,
    source: "user",
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

// Most exercises are resolved by the ported tables rather than by the user.
// The row has to say which, or a standard pair looks like a deliberate
// choice someone made.
test("a table-resolved row shows only its Garmin destination", () => {
  renderRow(mapping({ source: "automatic" }));
  expect(screen.getByText("Strength › Shoulder Press")).toBeVisible();
  expect(screen.queryByText(/Standard mapping/i)).toBeNull();
});

test("a user override is labelled as such, not as a standard mapping", () => {
  renderRow(mapping({ source: "user" }));
  expect(screen.getByText(/Your override/i)).toBeVisible();
  expect(screen.queryByText(/Standard mapping/i)).toBeNull();
});

test("an unmapped row says so and offers Map", () => {
  renderRow(
    mapping({
      mapped: false,
      unmapped: true,
      source: "",
      categoryName: null,
      subcategoryName: null,
    }),
  );
  expect(screen.getByText(/Unmapped/)).toBeVisible();
  expect(screen.getByRole("link", { name: "Map Landmine Press" })).toBeVisible();
});

test("a suggested row looks like every other unmapped row", () => {
  renderRow(
    mapping({
      mapped: false,
      unmapped: true,
      suggested: true,
      source: "",
      categoryName: "Squat",
      subcategoryName: "Back squat",
    }),
  );
  expect(screen.getByText("Unmapped")).toBeVisible();
  expect(screen.queryByText("shoulders")).toBeNull();
  expect(screen.queryByText("Suggestion ready")).toBeNull();
  const map = screen.getByRole("link", { name: "Map Landmine Press" });
  expect(map).toBeVisible();
  expect(map).toHaveAttribute("data-variant", "warning");
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
