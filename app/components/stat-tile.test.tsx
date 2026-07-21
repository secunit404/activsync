import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { StatTile } from "./stat-tile";

test("renders label and value", () => {
  render(<StatTile label="THIS WEEK" value="7h 42m" tone="neutral" />);
  expect(screen.getByText("THIS WEEK")).toBeInTheDocument();
  expect(screen.getByText("7h 42m")).toBeInTheDocument();
});

test.each([
  ["pending", "text-primary"],
  ["held", "text-warning"],
  ["published", "text-success"],
] as const)("tones the %s value distinctly from neutral", (tone, expectedClass) => {
  render(<StatTile label="PENDING" value="02" tone={tone} />);
  expect(screen.getByText("02")).toHaveClass(expectedClass);
});
