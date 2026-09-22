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

  expect(screen.getByText(/Found 12 activities over the last 30 days/)).toBeInTheDocument();
  expect(screen.getByText(/8 already existed on Strava/)).toBeInTheDocument();
  expect(screen.getByText(/4 are held for review/)).toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(onDismiss).toHaveBeenCalledOnce();
});

test("is not an alert region — informational only", () => {
  render(<CatchUpReport report={report} onDismiss={vi.fn()} />);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

test("singularizes activity/day counts of 1", () => {
  render(
    <CatchUpReport report={{ new: 1, held: 0, linked: 1, days: 1 }} onDismiss={vi.fn()} />,
  );
  expect(screen.getByText(/Found 1 activity over the last 1 day/)).toBeInTheDocument();
});
