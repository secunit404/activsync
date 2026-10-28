import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { Checkbox } from "./checkbox";

test("a partly-selected checkbox draws a dash and reports mixed", () => {
  render(<Checkbox aria-label="Select all" checked="indeterminate" />);
  const box = screen.getByRole("checkbox", { name: "Select all" });
  expect(box).toHaveAttribute("aria-checked", "mixed");
  expect(box.querySelector(".lucide-minus")).not.toBeNull();
  expect(box.querySelector(".lucide-check")).toBeNull();
});

test("a checked checkbox draws a check", () => {
  render(<Checkbox aria-label="Select one" checked />);
  const box = screen.getByRole("checkbox", { name: "Select one" });
  expect(box).toHaveAttribute("aria-checked", "true");
  expect(box.querySelector(".lucide-check")).not.toBeNull();
  expect(box.querySelector(".lucide-minus")).toBeNull();
});
