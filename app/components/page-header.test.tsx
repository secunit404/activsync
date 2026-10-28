import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { PageContainer, PageHeader } from "./page-header";

test("renders the title as the page heading", () => {
  render(<PageHeader title="Activities" description="Review your sync history." />);
  expect(screen.getByRole("heading", { level: 1, name: "Activities" })).toBeVisible();
  expect(screen.getByText("Review your sync history.")).toBeVisible();
});

// The rail carries the brand from md: up; phones have no rail, so the
// header shows the logo mark there instead of a wordmark eyebrow.
test("shows the logo mark, not a wordmark eyebrow", () => {
  render(<PageHeader title="Settings" description="Manage connections." />);
  const logo = screen.getByRole("img", { name: "ActivSync" });
  expect(logo).toHaveAttribute("src", "/favicon.ico");
  expect(logo.className).toContain("md:hidden");
  expect(screen.queryByLabelText("ActivSync")).not.toBeInTheDocument();
  expect(screen.queryByText(/activ/i)).not.toBeInTheDocument();
});

test("the logo sits in the heading's row, not a row of its own", () => {
  render(<PageHeader title="Settings" description="Manage connections." />);
  const logo = screen.getByRole("img", { name: "ActivSync" });
  expect(logo.parentElement).toBe(
    screen.getByRole("heading", { level: 1, name: "Settings" }).parentElement,
  );
});

test("renders an action beside the title when given", () => {
  render(
    <PageHeader title="Hevy" description="Workouts." action={<span>Connected</span>} />,
  );
  expect(screen.getByText("Connected")).toBeVisible();
});

test("omits the action slot entirely when none is given", () => {
  const { container } = render(<PageHeader title="Hevy" description="Workouts." />);
  expect(container.textContent).toBe("HevyWorkouts.");
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
