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
