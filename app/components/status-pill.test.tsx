import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { StatusPill } from "./status-pill";

test.each([
  ["pending", "PENDING"],
  ["held", "HELD"],
  ["published", "PUBLISHED"],
  ["missing", "MISSING"],
  ["excluded", "EXCLUDED"],
] as const)("renders %s", (status, label) => {
  render(<StatusPill status={status} />);
  expect(screen.getByText(label)).toBeInTheDocument();
});

test("the pill explains its status on hover, but not while publishing", () => {
  const { rerender } = render(<StatusPill status="held" />);
  expect(screen.getByText("HELD")).toHaveAttribute(
    "title",
    "Waits for you. Nothing reaches Strava until you publish.",
  );
  rerender(<StatusPill status="held" publishing />);
  expect(screen.getByText("PUBLISHING…")).not.toHaveAttribute("title");
});
