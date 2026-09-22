import { expect, test } from "vitest";

import {
  categorizeActivityType,
  groupActivityTypesByCategory,
} from "./activity-type-categories";

test("groups well-known Garmin type keys into their expected category", () => {
  expect(categorizeActivityType("running")).toBe("Running");
  expect(categorizeActivityType("trail_running")).toBe("Running");
  expect(categorizeActivityType("road_biking")).toBe("Cycling");
  expect(categorizeActivityType("open_water_swimming")).toBe("Swimming");
  expect(categorizeActivityType("golf")).toBe("Golf");
});

test("resolves overrides ahead of the generic keyword rules", () => {
  // "stair_climbing" and "sky_diving" would otherwise match the broader
  // "climb"/"diving" keyword rules and land in the wrong bucket.
  expect(categorizeActivityType("stair_climbing")).toBe("Strength & fitness");
  expect(categorizeActivityType("sky_diving")).toBe("Motor sports");
});

test("falls back to Other for keys with no matching keyword", () => {
  expect(categorizeActivityType("archery")).toBe("Other");
  expect(categorizeActivityType("some_future_garmin_type")).toBe("Other");
});

test("groups items, preserving intra-group order and sorting groups by name", () => {
  const groups = groupActivityTypesByCategory([
    { typeKey: "swimming", label: "Swimming" },
    { typeKey: "running", label: "Running" },
    { typeKey: "trail_running", label: "Trail Running" },
    { typeKey: "archery", label: "Archery" },
  ]);

  expect(groups.map((group) => group.category)).toEqual([
    "Other",
    "Running",
    "Swimming",
  ]);
  const running = groups.find((group) => group.category === "Running");
  expect(running?.items.map((item) => item.typeKey)).toEqual([
    "running",
    "trail_running",
  ]);
});
