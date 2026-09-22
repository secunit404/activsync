import { expect, test } from "vitest";

import { isExcludableStatus, isPublishableStatus } from "./activity-metrics";

// Mirrors api_routes.py's exclude endpoint, which answers 409 "Only pending,
// held, or missing activities can be excluded" for anything else.
test("only pending, held and missing activities are excludable", () => {
  expect(isExcludableStatus("pending")).toBe(true);
  expect(isExcludableStatus("held")).toBe(true);
  expect(isExcludableStatus("missing")).toBe(true);
  expect(isExcludableStatus("published")).toBe(false);
  expect(isExcludableStatus("excluded")).toBe(false);
});

// The two predicates answer different questions and are kept as separate
// functions for that reason, but they must not silently drift apart without
// someone noticing — this test is the tripwire.
test("excludable and publishable agree on every status today", () => {
  for (const status of ["pending", "held", "published", "missing", "excluded"] as const) {
    expect(isExcludableStatus(status)).toBe(isPublishableStatus(status));
  }
});
