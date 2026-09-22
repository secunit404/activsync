import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { BulkActionBar } from "./bulk-action-bar";

function renderBar(overrides: Partial<React.ComponentProps<typeof BulkActionBar>> = {}) {
  const props = {
    count: 2,
    excludableCount: 2,
    publishableCount: 2,
    onClear: vi.fn(),
    onExclude: vi.fn(),
    onPublish: vi.fn(),
    busy: false,
    ...overrides,
  };
  render(<BulkActionBar {...props} />);
  return props;
}

test("renders nothing when nothing is selected", () => {
  const { container } = render(
    <BulkActionBar
      count={0}
      excludableCount={0}
      publishableCount={0}
      onClear={vi.fn()}
      onExclude={vi.fn()}
      onPublish={vi.fn()}
      busy={false}
    />,
  );
  expect(container).toBeEmptyDOMElement();
});

// Names were removed deliberately: with a large selection the joined list
// pushed the desktop bar to an unusable width. The count is the whole
// summary now.
test("shows the count and never lists the selected activity names", () => {
  renderBar();
  expect(screen.getAllByText("2 selected").length).toBeGreaterThan(0);
  expect(screen.queryByText(/Morning Run/)).toBeNull();
});

// Only one element carries the testid the E2E spec queries with a single
// (strict-mode) locator across every viewport project — see the component's
// own docstring for why the two layouts live inside it rather than as two
// sibling elements.
test("is a single element in the DOM, not two viewport-swapped copies", () => {
  renderBar();
  expect(screen.getAllByTestId("bulk-action-bar")).toHaveLength(1);
});

test("Clear calls onClear", async () => {
  const props = renderBar();
  await userEvent.click(screen.getAllByRole("button", { name: "Clear" })[0]);
  expect(props.onClear).toHaveBeenCalledOnce();
});

test("Exclude calls onExclude", async () => {
  const props = renderBar();
  await userEvent.click(screen.getAllByRole("button", { name: /Exclude/ })[0]);
  expect(props.onExclude).toHaveBeenCalledOnce();
});

test("Publish shows the count and calls onPublish", async () => {
  const props = renderBar();
  const buttons = screen.getAllByRole("button", { name: "Publish 2" });
  expect(buttons.length).toBeGreaterThan(0);
  await userEvent.click(buttons[0]);
  expect(props.onPublish).toHaveBeenCalledOnce();
});

test("disables Exclude and Publish while busy", () => {
  renderBar({ busy: true });
  for (const button of screen.getAllByRole("button", { name: /Exclude/ })) {
    expect(button).toBeDisabled();
  }
  for (const button of screen.getAllByRole("button", { name: /Publish/ })) {
    expect(button).toBeDisabled();
  }
});

// A broken Strava connection means Publish would fail immediately on
// submit (see AttentionBanner's copy) — Exclude is a purely local write and
// stays clickable regardless.
test("publishDisabled disables only Publish, with an explanatory title", () => {
  renderBar({ publishDisabled: true });
  for (const button of screen.getAllByRole("button", { name: /Exclude/ })) {
    expect(button).not.toBeDisabled();
  }
  for (const button of screen.getAllByRole("button", { name: /Publish/ })) {
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("title", "Reconnect Strava to resume publishing.");
  }
});

// A mixed selection acts on the subset the server would accept rather than
// blocking on one ineligible row — mirrors api_routes.py's status guards, so
// the UI can no longer produce a 409 the user reads as a failed action.
test("Exclude counts only the excludable rows in the selection", () => {
  renderBar({ count: 5, excludableCount: 3, publishableCount: 3 });
  expect(screen.getAllByRole("button", { name: "Exclude 3" }).length).toBeGreaterThan(0);
  // The raw selection size still reads honestly.
  expect(screen.getAllByText("5 selected").length).toBeGreaterThan(0);
});

test("Exclude is disabled when nothing selected can be excluded", () => {
  renderBar({ count: 2, excludableCount: 0, publishableCount: 0 });
  for (const button of screen.getAllByRole("button", { name: /Exclude/ })) {
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute(
      "title",
      "None of the selected activities can be excluded.",
    );
  }
});

test("Publish is disabled when nothing selected can be published", () => {
  renderBar({ count: 2, excludableCount: 2, publishableCount: 0 });
  for (const button of screen.getAllByRole("button", { name: /Publish/ })) {
    expect(button).toBeDisabled();
  }
  // Exclude is unaffected — the two subsets are independent.
  for (const button of screen.getAllByRole("button", { name: /Exclude/ })) {
    expect(button).not.toBeDisabled();
  }
});

test("publishDisabled is false by default, so Publish stays enabled", () => {
  renderBar();
  for (const button of screen.getAllByRole("button", { name: /Publish/ })) {
    expect(button).not.toBeDisabled();
    expect(button).not.toHaveAttribute("title");
  }
});
