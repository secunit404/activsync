import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { TypePill } from "./type-pill";

test("renders a short type as-is, at the 12px reading floor", () => {
  render(<TypePill type="running" />);
  expect(screen.getByText("RUNNING")).toHaveClass("text-xs");
});

test("truncates a long type and keeps the raw value discoverable via title", () => {
  render(<TypePill type="stand_up_paddleboarding_v2" />);
  const pill = screen.getByTitle("stand_up_paddleboarding_v2");
  expect(pill).toHaveClass("truncate");
  expect(pill).toHaveClass("max-w-[10rem]");
});
