import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";

import { Hint } from "./hint";

function renderHint() {
  render(<Hint label="Garmin poll interval">How often ActivSync checks Garmin.</Hint>);
  return screen.getByRole("button", { name: "About Garmin poll interval" });
}

test("hides the note until the icon is used", () => {
  renderHint();
  expect(screen.queryByTestId("hint-content")).toBeNull();
});

// The reason this is a popover and not a tooltip: a phone has no hover, and a
// Radix tooltip would leave every description unreachable there. A tap has to
// open it and leave it open — a touch `pointerenter` fires on tap too, so if
// hover were not ignored for touch pointers it would open and instantly close.
test("a tap opens the note and leaves it open", async () => {
  const user = userEvent.setup();
  const trigger = renderHint();

  await user.pointer({ keys: "[TouchA]", target: trigger });

  expect(await screen.findByTestId("hint-content")).toHaveTextContent(
    "How often ActivSync checks Garmin.",
  );
});

test("hovering opens it and leaving closes it again", async () => {
  const user = userEvent.setup();
  const trigger = renderHint();

  await user.hover(trigger);
  expect(await screen.findByTestId("hint-content")).toBeInTheDocument();

  await user.unhover(trigger);
  expect(screen.queryByTestId("hint-content")).toBeNull();
});

test("names the setting it explains, so the icon is not unlabelled", () => {
  const trigger = renderHint();
  expect(trigger).toHaveAccessibleName("About Garmin poll interval");
});

test("is reachable and openable from the keyboard", async () => {
  const user = userEvent.setup();
  renderHint();

  await user.tab();
  expect(screen.getByRole("button", { name: "About Garmin poll interval" })).toHaveFocus();

  await user.keyboard("{Enter}");

  expect(await screen.findByTestId("hint-content")).toBeInTheDocument();
});
