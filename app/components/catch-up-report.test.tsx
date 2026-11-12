import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { CatchUpReport } from "./catch-up-report";

const report = { new: 12, held: 4, linked: 8, days: 30 };

test("renders nothing when there is no report", () => {
  const { container } = render(<CatchUpReport report={null} onDismiss={vi.fn()} />);
  expect(container).toBeEmptyDOMElement();
});

test("summarizes the reconnect catch-up and offers dismiss", async () => {
  const onDismiss = vi.fn();
  render(<CatchUpReport report={report} onDismiss={onDismiss} />);

  expect(
    screen.getByText(
      "Found 12 activities from the last 30 days. 8 were already on Strava. 4 are held for your review.",
    ),
  ).toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(onDismiss).toHaveBeenCalledOnce();
});

test("is not an alert region — informational only", () => {
  render(<CatchUpReport report={report} onDismiss={vi.fn()} />);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

test("counts of 1 read in the singular", () => {
  render(
    <CatchUpReport report={{ new: 2, held: 1, linked: 1, days: 1 }} onDismiss={vi.fn()} />,
  );
  expect(
    screen.getByText(
      "Found 2 activities from the last day. 1 was already on Strava. 1 is held for your review.",
    ),
  ).toBeInTheDocument();
});

test("a count of zero is left out rather than stated", () => {
  render(
    <CatchUpReport report={{ new: 1, held: 1, linked: 0, days: 3 }} onDismiss={vi.fn()} />,
  );
  expect(
    screen.getByText("Found 1 activity from the last 3 days. 1 is held for your review."),
  ).toBeInTheDocument();
});
