import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { ResponsiveOverlay } from "./responsive-overlay";

function Harness({
  onOpenChange = () => {},
  open = true,
  mobile,
  description,
}: {
  onOpenChange?: (open: boolean) => void;
  open?: boolean;
  mobile?: "cover" | "sheet";
  description?: string;
}) {
  return (
    <ResponsiveOverlay
      open={open}
      title="Details"
      description={description}
      mobile={mobile}
      onOpenChange={onOpenChange}
      footer={<button>Publish</button>}
    >
      <p>Body</p>
    </ResponsiveOverlay>
  );
}

test("renders as a dialog with an accessible name", () => {
  render(<Harness />);
  expect(screen.getByRole("dialog", { name: "Details" })).toBeInTheDocument();
});

test("defaults to the 560px desktop width", () => {
  render(
    <ResponsiveOverlay open onOpenChange={vi.fn()} title="Narrow">
      <p>Body</p>
    </ResponsiveOverlay>,
  );
  expect(screen.getByRole("dialog").className).toContain("md:max-w-[560px]");
});

// Backfill lists scannable rows — checkbox, title, subtitle and an action.
// At 560px every row truncated.
test("size=wide opts into the 760px desktop width", () => {
  render(
    <ResponsiveOverlay open onOpenChange={vi.fn()} title="Wide" size="wide">
      <p>Body</p>
    </ResponsiveOverlay>,
  );
  const classes = screen.getByRole("dialog").className;
  expect(classes).toContain("md:max-w-[760px]");
  expect(classes).not.toContain("md:max-w-[560px]");
});

test("renders the sticky footer", () => {
  render(<Harness />);
  const footer = screen.getByTestId("responsive-overlay-footer");
  expect(screen.getByRole("button", { name: "Publish" })).toBeInTheDocument();
  expect(footer).toHaveClass("sticky", "bottom-0", "bg-card");
});

test("omits the footer region when no footer is given", () => {
  render(
    <ResponsiveOverlay open title="Details" onOpenChange={() => {}}>
      <p>Body</p>
    </ResponsiveOverlay>,
  );
  expect(screen.queryByTestId("responsive-overlay-footer")).not.toBeInTheDocument();
});

test("escape requests close", async () => {
  const onOpenChange = vi.fn();
  render(<Harness onOpenChange={onOpenChange} />);
  await userEvent.keyboard("{Escape}");
  expect(onOpenChange).toHaveBeenCalledWith(false);
});

test("the close button requests close", async () => {
  const onOpenChange = vi.fn();
  render(<Harness onOpenChange={onOpenChange} />);
  await userEvent.click(screen.getByRole("button", { name: "Close" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
});

test("keeps focus inside the overlay", async () => {
  const user = userEvent.setup();
  render(
    <>
      <button>Outside</button>
      <Harness />
    </>,
  );
  const dialog = screen.getByRole("dialog", { name: "Details" });
  for (let index = 0; index < 6; index += 1) {
    await user.tab();
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
  }
});

test("wires the description as the accessible description", () => {
  render(<Harness description="One activity" />);
  expect(screen.getByRole("dialog", { name: "Details" })).toHaveAccessibleDescription(
    "One activity",
  );
});

test("leaves the dialog undescribed when no description is given", () => {
  render(<Harness />);
  expect(screen.getByRole("dialog", { name: "Details" })).not.toHaveAttribute(
    "aria-describedby",
  );
});

test("renders nothing while closed", () => {
  render(<Harness open={false} />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

test("the body scrolls with a hidden scrollbar", () => {
  render(<Harness />);
  const body = screen.getByTestId("responsive-overlay-body");
  expect(body).toContainElement(screen.getByText("Body"));
  expect(body).toHaveClass("overlay-scroll", "overflow-y-auto");
});

test("defaults the mobile presentation to a full-screen cover", () => {
  render(<Harness />);
  expect(screen.getByRole("dialog", { name: "Details" })).toHaveAttribute(
    "data-mobile",
    "cover",
  );
});

test("takes a bottom-sheet mobile presentation", () => {
  render(<Harness mobile="sheet" />);
  expect(screen.getByRole("dialog", { name: "Details" })).toHaveAttribute(
    "data-mobile",
    "sheet",
  );
});

test("defaults to the dialog role", () => {
  render(<Harness />);
  expect(screen.getByRole("dialog", { name: "Details" })).toBeInTheDocument();
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
});

test("takes an alertdialog role for destructive confirmations", () => {
  render(
    <ResponsiveOverlay open title="Details" onOpenChange={() => {}} role="alertdialog">
      <p>Body</p>
    </ResponsiveOverlay>,
  );
  expect(screen.getByRole("alertdialog", { name: "Details" })).toBeInTheDocument();
});

test("omits the scrolling body region entirely when no children are given", () => {
  render(
    <ResponsiveOverlay open title="Details" onOpenChange={() => {}} />,
  );
  expect(screen.queryByTestId("responsive-overlay-body")).not.toBeInTheDocument();
});

test("forwards onOpenAutoFocus so a consumer can redirect initial focus", () => {
  const onOpenAutoFocus = vi.fn((event: Event) => event.preventDefault());
  render(
    <ResponsiveOverlay
      open
      title="Details"
      onOpenChange={() => {}}
      onOpenAutoFocus={onOpenAutoFocus}
    >
      <p>Body</p>
    </ResponsiveOverlay>,
  );
  expect(onOpenAutoFocus).toHaveBeenCalled();
});
