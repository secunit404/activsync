import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { MfaCodeInput } from "./mfa-code-input";

test("typing six digits fires onComplete once", async () => {
  const onComplete = vi.fn();
  render(<MfaCodeInput length={6} onComplete={onComplete} />);
  await userEvent.type(screen.getByRole("textbox", { name: /digit 1/i }), "123456");
  expect(onComplete).toHaveBeenCalledExactlyOnceWith("123456");
});

test("backspace on an empty box moves focus back", async () => {
  render(<MfaCodeInput length={6} onComplete={vi.fn()} />);
  const second = screen.getByRole("textbox", { name: /digit 2/i });
  second.focus();
  await userEvent.keyboard("{Backspace}");
  expect(screen.getByRole("textbox", { name: /digit 1/i })).toHaveFocus();
});

test("pasting a code fills every box", async () => {
  const onComplete = vi.fn();
  render(<MfaCodeInput length={6} onComplete={onComplete} />);
  screen.getByRole("textbox", { name: /digit 1/i }).focus();
  await userEvent.paste("654321");
  expect(onComplete).toHaveBeenCalledWith("654321");
});

test("pasting fills every box's visible value, not just the first", async () => {
  render(<MfaCodeInput length={6} onComplete={vi.fn()} />);
  screen.getByRole("textbox", { name: /digit 1/i }).focus();
  await userEvent.paste("654321");
  expect(screen.getByRole("textbox", { name: /digit 1/i })).toHaveValue("6");
  expect(screen.getByRole("textbox", { name: /digit 6/i })).toHaveValue("1");
});

test("rejects a non-numeric character instead of displaying it", async () => {
  const onComplete = vi.fn();
  render(<MfaCodeInput length={6} onComplete={onComplete} />);
  await userEvent.type(screen.getByRole("textbox", { name: /digit 1/i }), "a");
  expect(screen.getByRole("textbox", { name: /digit 1/i })).toHaveValue("");
  expect(onComplete).not.toHaveBeenCalled();
});

test("does not fire onComplete again after backspacing and retyping the final digit", async () => {
  const onComplete = vi.fn();
  render(<MfaCodeInput length={6} onComplete={onComplete} />);
  await userEvent.type(screen.getByRole("textbox", { name: /digit 1/i }), "123456");
  expect(onComplete).toHaveBeenCalledTimes(1);
  await userEvent.type(screen.getByRole("textbox", { name: /digit 6/i }), "{Backspace}7");
  expect(onComplete).toHaveBeenCalledTimes(2);
  expect(onComplete).toHaveBeenLastCalledWith("123457");
});

test("the first box carries inputMode and one-time-code autocomplete", () => {
  render(<MfaCodeInput length={6} onComplete={vi.fn()} />);
  const first = screen.getByRole("textbox", { name: /digit 1/i });
  expect(first).toHaveAttribute("inputMode", "numeric");
  expect(first).toHaveAttribute("autoComplete", "one-time-code");
});

test("disabled boxes reject input", () => {
  render(<MfaCodeInput length={6} onComplete={vi.fn()} disabled />);
  expect(screen.getByRole("textbox", { name: /digit 1/i })).toBeDisabled();
});
