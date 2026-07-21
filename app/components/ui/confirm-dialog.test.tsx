import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { ConfirmDialog } from "./confirm-dialog";

function Harness({
  open = true,
  pending = false,
  onOpenChange = () => {},
  onConfirm = () => {},
}: {
  open?: boolean;
  pending?: boolean;
  onOpenChange?: (open: boolean) => void;
  onConfirm?: () => void;
}) {
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Disconnect Strava?"
      description="Publishing pauses, but your activity list is kept."
      confirmLabel="Disconnect"
      pendingLabel="Disconnecting…"
      pending={pending}
      onConfirm={onConfirm}
    />
  );
}

test("renders as an alertdialog with the accessible name and description wired", () => {
  render(<Harness />);
  const dialog = screen.getByRole("alertdialog", { name: "Disconnect Strava?" });
  expect(dialog).toHaveAccessibleDescription(
    "Publishing pauses, but your activity list is kept.",
  );
});

test("initial focus lands on the cancel button, not the destructive one", async () => {
  render(<Harness />);
  await vi.waitFor(() =>
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus(),
  );
});

test("cancel calls onOpenChange(false) and never onConfirm", async () => {
  const onOpenChange = vi.fn();
  const onConfirm = vi.fn();
  render(<Harness onOpenChange={onOpenChange} onConfirm={onConfirm} />);

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

  expect(onOpenChange).toHaveBeenCalledWith(false);
  expect(onConfirm).not.toHaveBeenCalled();
});

test("escape calls onOpenChange(false) and never onConfirm", async () => {
  const onOpenChange = vi.fn();
  const onConfirm = vi.fn();
  render(<Harness onOpenChange={onOpenChange} onConfirm={onConfirm} />);

  await userEvent.keyboard("{Escape}");

  expect(onOpenChange).toHaveBeenCalledWith(false);
  expect(onConfirm).not.toHaveBeenCalled();
});

test("the destructive button calls onConfirm", async () => {
  const onConfirm = vi.fn();
  render(<Harness onConfirm={onConfirm} />);

  await userEvent.click(screen.getByRole("button", { name: "Disconnect" }));

  expect(onConfirm).toHaveBeenCalledTimes(1);
});

test("the destructive button is visibly destructive and disabled while pending, showing the pending label", () => {
  render(<Harness pending />);
  const confirmButton = screen.getByRole("button", { name: /Disconnecting/ });
  expect(confirmButton).toBeDisabled();
  expect(confirmButton).toHaveAttribute("data-variant", "destructive");
});

test("clicking the pending confirm button does not fire onConfirm again", async () => {
  const onConfirm = vi.fn();
  render(<Harness pending onConfirm={onConfirm} />);

  await userEvent.click(screen.getByRole("button", { name: /Disconnecting/ }));

  expect(onConfirm).not.toHaveBeenCalled();
});

test("renders nothing while closed", () => {
  render(<Harness open={false} />);
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
});
