import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";

import { Button, buttonVariants } from "./button";

const VARIANTS = [
  "default",
  "outline",
  "secondary",
  "ghost",
  "destructive",
  "warning",
  "link",
] as const;

// app.css configures no `dark` variant, so Tailwind resolves `dark:` via
// prefers-color-scheme only — which never matches on a light-mode OS even
// though this app is dark-only (`color-scheme: dark` plus explicit token
// values). Any `dark:` utility here is therefore dead styling, and was why
// `outline` rendered near-invisibly on `--card`.
test("no variant relies on a dark: utility", () => {
  for (const variant of VARIANTS) {
    expect(buttonVariants({ variant })).not.toMatch(/(^|\s)dark:/);
  }
});

test("outline has a visible resting surface, not the page background", () => {
  expect(buttonVariants({ variant: "outline" })).not.toMatch(
    /(^|\s)bg-background(\s|$)/,
  );
});

test("default hover resolves lighter, not toward the dark background", () => {
  // `hover:bg-primary/80` composited over `--background` darkens the button,
  // which reads as disabled. Mix toward `--foreground` instead.
  const classes = buttonVariants({ variant: "default" });
  expect(classes).not.toContain("hover:bg-primary/80");
  expect(classes).toContain("color-mix");
});

test("xl is the 44px dialog-footer size", () => {
  expect(buttonVariants({ size: "xl" })).toContain("h-11");
});

// cva silently ignores an unknown variant key and falls through to the base
// classes, so asserting only that the button renders would pass even if
// `warning` did not exist. Assert on the emitted classes instead.
test("warning variant is tinted, never a solid --warning fill", () => {
  const classes = buttonVariants({ variant: "warning" });
  expect(classes).toContain("text-warning");
  // #f2c14e behind near-black text was the unreadable pairing in
  // backfill-row.tsx — a solid fill must not come back.
  expect(classes).not.toMatch(/(^|\s)bg-warning(\s|$)/);
});

test("warning variant renders and is clickable", () => {
  render(<Button variant="warning">Map</Button>);
  expect(screen.getByRole("button", { name: "Map" })).toBeEnabled();
});
