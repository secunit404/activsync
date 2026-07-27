import { act, fireEvent, render, screen } from "@testing-library/react";
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

// Radix's own default is the first focusable child — the close button, or a
// form's first field — which opened every overlay with a focus ring on a
// control nobody chose.
test("opens with focus on the panel, not the close button", () => {
  render(<Harness />);
  const dialog = screen.getByRole("dialog", { name: "Details" });
  expect(document.activeElement).toBe(dialog);
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

test("the body is the scroll region", () => {
  render(<Harness />);
  const body = screen.getByTestId("responsive-overlay-body");
  expect(body).toContainElement(screen.getByText("Body"));
  expect(body).toHaveClass("overlay-scroll", "overflow-y-auto");
});

function dragHandle(handle: HTMLElement, distance: number) {
  fireEvent.pointerDown(handle, { clientY: 0, pointerId: 1 });
  fireEvent.pointerMove(handle, { clientY: distance, pointerId: 1 });
  fireEvent.pointerUp(handle, { clientY: distance, pointerId: 1 });
}

test("dragging the sheet handle past the threshold requests close", () => {
  const onOpenChange = vi.fn();
  render(<Harness mobile="sheet" onOpenChange={onOpenChange} />);
  dragHandle(screen.getByTestId("responsive-overlay-handle"), 150);
  expect(onOpenChange).toHaveBeenCalledWith(false);
});

// A stray swipe on the handle must not close a sheet the user is reading.
test("a short drag springs back instead of dismissing", () => {
  const onOpenChange = vi.fn();
  render(<Harness mobile="sheet" onOpenChange={onOpenChange} />);
  dragHandle(screen.getByTestId("responsive-overlay-handle"), 40);
  expect(onOpenChange).not.toHaveBeenCalled();
  expect(screen.getByRole("dialog")).toHaveStyle({ "--sheet-drag": "0px" });
});

// Resetting the translate before the exit animation started snapped the sheet
// back to its resting position for a frame, which read as a flicker. The
// offset has to survive the dismissal so the slide-out continues from it.
test("a dismissing drag keeps its offset for the exit animation", () => {
  render(<Harness mobile="sheet" />);
  dragHandle(screen.getByTestId("responsive-overlay-handle"), 150);
  expect(screen.getByRole("dialog")).toHaveStyle({ "--sheet-drag": "150px" });
});

// …and is cleared before the next open, or the sheet slides in pre-pushed.
test("re-opening starts from a fresh, untranslated sheet", () => {
  const { rerender } = render(<Harness mobile="sheet" />);
  dragHandle(screen.getByTestId("responsive-overlay-handle"), 150);
  rerender(<Harness mobile="sheet" open={false} />);
  rerender(<Harness mobile="sheet" />);
  expect(screen.getByRole("dialog")).toHaveStyle({ "--sheet-drag": "0px" });
});

// React treats `pointermove` as continuous priority, so a fast drag can batch
// every move without an intervening render. Dispatching the whole gesture
// inside one `act` reproduces that: a handler reading `dragY` from the render
// closure still sees 0 at release and refuses to dismiss.
test("dismisses on a fast drag that never re-renders mid-gesture", () => {
  const onOpenChange = vi.fn();
  render(<Harness mobile="sheet" onOpenChange={onOpenChange} />);
  const handle = screen.getByTestId("responsive-overlay-handle");
  const event = (type: string, clientY: number) =>
    new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, clientY });
  act(() => {
    handle.dispatchEvent(event("pointerdown", 0));
    handle.dispatchEvent(event("pointermove", 200));
    handle.dispatchEvent(event("pointerup", 200));
  });
  expect(onOpenChange).toHaveBeenCalledWith(false);
});

test("the sheet tracks the pointer while dragging", () => {
  render(<Harness mobile="sheet" />);
  const handle = screen.getByTestId("responsive-overlay-handle");
  fireEvent.pointerDown(handle, { clientY: 20, pointerId: 1 });
  fireEvent.pointerMove(handle, { clientY: 90, pointerId: 1 });
  expect(screen.getByRole("dialog")).toHaveStyle({ "--sheet-drag": "70px" });
});

// Dragging up has nowhere to go — the sheet is already at its full height.
test("dragging up does not detach the sheet from the bottom edge", () => {
  render(<Harness mobile="sheet" />);
  const handle = screen.getByTestId("responsive-overlay-handle");
  fireEvent.pointerDown(handle, { clientY: 200, pointerId: 1 });
  fireEvent.pointerMove(handle, { clientY: 40, pointerId: 1 });
  expect(screen.getByRole("dialog")).toHaveStyle({ "--sheet-drag": "0px" });
});

// The handle is the only drag surface, and it is `md:hidden` — that is what
// keeps the gesture off desktop without a JS breakpoint.
test("the cover presentation has no drag handle", () => {
  render(<Harness mobile="cover" />);
  expect(screen.queryByTestId("responsive-overlay-handle")).not.toBeInTheDocument();
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
