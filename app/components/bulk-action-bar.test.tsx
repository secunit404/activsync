import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { BulkActionBar } from "./bulk-action-bar";

function renderBar(overrides: Partial<React.ComponentProps<typeof BulkActionBar>> = {}) {
  const props = {
    count: 2,
    names: ["Morning Run", "Push Day"],
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
      names={[]}
      onClear={vi.fn()}
      onExclude={vi.fn()}
      onPublish={vi.fn()}
      busy={false}
    />,
  );
  expect(container).toBeEmptyDOMElement();
});

test("shows the count and the selected names", () => {
  renderBar();
  expect(screen.getAllByText("2 selected").length).toBeGreaterThan(0);
  expect(screen.getAllByText("Morning Run, Push Day").length).toBeGreaterThan(0);
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
  await userEvent.click(screen.getAllByRole("button", { name: "Exclude" })[0]);
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
  for (const button of screen.getAllByRole("button", { name: "Exclude" })) {
    expect(button).toBeDisabled();
  }
  for (const button of screen.getAllByRole("button", { name: /Publish/ })) {
    expect(button).toBeDisabled();
  }
});
