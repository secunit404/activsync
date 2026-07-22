import { render, screen } from "@testing-library/react";
import { createRoutesStub } from "react-router";
import { expect, test, vi } from "vitest";

import { BackfillRow, type BackfillItem } from "./backfill-row";

function item(overrides: Partial<BackfillItem> = {}): BackfillItem {
  return {
    hevyId: "w-1",
    title: "Old ring circuit",
    startTime: "2026-07-01T10:00:00Z",
    action: "needs_mapping",
    twinActivityId: null,
    missingTemplateIds: ["tpl-dev-custom"],
    ...overrides,
  };
}

function renderRow(value: BackfillItem) {
  const Stub = createRoutesStub([
    {
      path: "/",
      Component: () => (
        <ul>
          <BackfillRow item={value} selected={false} onToggle={vi.fn()} />
        </ul>
      ),
    },
  ]);
  render(<Stub initialEntries={["/"]} />);
}

// The mapping editor is nested under backfill so the preview survives the
// round trip — see routes.ts.
test("a locked row with a linkable template offers an enabled Map action", () => {
  renderRow(item());
  const link = screen.getByRole("link", { name: /Map/ });
  expect(link).toHaveAttribute("href", "/hevy/mapping/tpl-dev-custom");
});

// The dead-end case: needs_mapping with no template id to link to. Same
// control, disabled — not a differently-styled second badge.
test("a locked row with no linkable template shows the same control, disabled", () => {
  renderRow(item({ missingTemplateIds: [] }));
  expect(screen.queryByRole("link", { name: /Map/ })).toBeNull();
  const button = screen.getByRole("button", { name: /Map/ });
  expect(button).toBeDisabled();
  expect(button).toHaveAttribute(
    "title",
    "This exercise has no linkable template — it can't be fixed from here.",
  );
});

test("an unlocked row shows its kind badge and no Map control", () => {
  renderRow(
    item({ action: "linked_existing", twinActivityId: 42, missingTemplateIds: [] }),
  );
  expect(screen.getByText("Link")).toBeVisible();
  expect(screen.queryByRole("link", { name: /Map/ })).toBeNull();
  expect(screen.queryByRole("button", { name: /Map/ })).toBeNull();
});
