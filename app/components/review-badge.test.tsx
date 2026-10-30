import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { ReviewBadge, reviewBadgeText, reviewNavLabel } from "./review-badge";

test("caps the displayed count at 99+", () => {
  expect(reviewBadgeText(7)).toBe("7");
  expect(reviewBadgeText(99)).toBe("99");
  expect(reviewBadgeText(100)).toBe("99+");
});

test("names the count in the link label, singular and plural", () => {
  expect(reviewNavLabel("Activities", 0)).toBe("Activities");
  expect(reviewNavLabel("Activities", 1)).toBe("Activities, 1 needs review");
  expect(reviewNavLabel("Activities", 3)).toBe("Activities, 3 need review");
  expect(reviewNavLabel("Activities", 250)).toBe("Activities, 250 need review");
});

test("renders nothing at zero and is hidden from assistive tech otherwise", () => {
  const { rerender } = render(<ReviewBadge count={0} />);
  expect(screen.queryByTestId("review-badge")).toBeNull();

  rerender(<ReviewBadge count={4} />);
  const badge = screen.getByTestId("review-badge");
  expect(badge).toHaveTextContent("4");
  expect(badge).toHaveAttribute("aria-hidden", "true");
  expect(badge).toHaveClass("bg-warning");
});
