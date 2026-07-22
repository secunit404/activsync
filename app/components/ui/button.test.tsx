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

// app.css sets a base `a { color: var(--activ) }`. A variant that declares
// no text colour of its own inherits it, so the same button rendered
// `asChild` around a <Link> came out blue while its plain-<button> sibling
// did not. Every variant must state its own colour.
test("every variant declares its own text colour", () => {
  // Only the variant's own classes, and only colour utilities — the shared
  // base string carries `text-sm`, which would satisfy a bare /text-/ match
  // without saying anything about colour.
  const base = buttonVariants({ variant: null as never });
  for (const variant of VARIANTS) {
    const own = buttonVariants({ variant })
      .split(/\s+/)
      .filter((cls) => !base.split(/\s+/).includes(cls));
    expect(
      own.some((cls) => /^text-(?!xs$|sm$|base$|lg$|xl$|\[)/.test(cls)),
      `variant "${variant}" sets no text colour (${own.join(" ")}), so an asChild <a> falls back to the base link colour`,
    ).toBe(true);
  }
});
