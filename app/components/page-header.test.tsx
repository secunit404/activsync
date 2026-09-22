import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { PageContainer, PageHeader } from "./page-header";

test("renders the title as the page heading", () => {
  render(<PageHeader title="Activities" description="Review your sync history." />);
  expect(screen.getByRole("heading", { level: 1, name: "Activities" })).toBeVisible();
  expect(screen.getByText("Review your sync history.")).toBeVisible();
});

test("shows the ActivSync eyebrow by default", () => {
  render(<PageHeader title="Settings" description="Manage connections." />);
  // AppBrand splits the wordmark across two coloured spans, so match its
  // aria-label rather than a single text node.
  expect(screen.getByLabelText("ActivSync")).toBeVisible();
});

test("renders an action beside the title when given", () => {
  render(
    <PageHeader title="Hevy" description="Workouts." action={<span>Connected</span>} />,
  );
  expect(screen.getByText("Connected")).toBeVisible();
});

test("omits the action slot entirely when none is given", () => {
  const { container } = render(<PageHeader title="Hevy" description="Workouts." />);
  expect(container.textContent).toBe("ActivSyncHevyWorkouts.");
});

test("the container constrains and centres its children", () => {
  const { container } = render(<PageContainer>body</PageContainer>);
  expect(container.firstElementChild?.className).toContain("max-w-6xl");
  expect(container.firstElementChild?.className).toContain("mx-auto");
});

test("the container merges an extra className rather than dropping it", () => {
  const { container } = render(<PageContainer className="grid gap-6">body</PageContainer>);
  const classes = container.firstElementChild?.className ?? "";
  expect(classes).toContain("max-w-6xl");
  expect(classes).toContain("gap-6");
});
