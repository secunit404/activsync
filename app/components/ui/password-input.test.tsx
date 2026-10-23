import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";

import { PasswordInput } from "./password-input";

function renderField() {
  render(
    <form>
      <label htmlFor="pw">Password</label>
      <PasswordInput id="pw" autoComplete="current-password" defaultValue="" />
    </form>,
  );
  return screen.getByLabelText("Password");
}

test("starts hidden with a show toggle that never submits", () => {
  const input = renderField();
  const toggle = screen.getByRole("button", { name: "Show password" });

  expect(input).toHaveAttribute("type", "password");
  expect(input).toHaveAttribute("autocomplete", "current-password");
  expect(toggle).toHaveAttribute("type", "button");
  expect(toggle).toHaveAttribute("aria-pressed", "false");
});

test("toggles between password and text and back", async () => {
  const input = renderField();

  await userEvent.click(screen.getByRole("button", { name: "Show password" }));
  expect(input).toHaveAttribute("type", "text");
  const hide = screen.getByRole("button", { name: "Hide password" });
  expect(hide).toHaveAttribute("aria-pressed", "true");

  await userEvent.click(hide);
  expect(input).toHaveAttribute("type", "password");
  expect(screen.getByRole("button", { name: "Show password" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
});

test("keeps the value and the focus when toggled", async () => {
  const input = renderField();

  await userEvent.type(input, "a long passphrase");
  await userEvent.click(screen.getByRole("button", { name: "Show password" }));

  expect(input).toHaveValue("a long passphrase");
  expect(input).toHaveFocus();
});
