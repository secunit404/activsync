import { expect, test } from "vitest";

import { activityExplanation, rowReason, statusDescription, waitingReason } from "./activity-status";

test("a held row from before hold reasons were stored reads as a category hold", () => {
  expect(waitingReason({ publishStatus: "held", holdReason: null })).toBe(
    "Auto-sync is off for this type",
  );
});

test("statuses that need nothing from the owner have no waiting reason", () => {
  for (const publishStatus of ["pending", "published", "excluded"] as const) {
    expect(waitingReason({ publishStatus, holdReason: null })).toBeNull();
  }
});

test("pending names the Strava check interval when it is known", () => {
  expect(statusDescription("pending")).toBe("Publishes to Strava on its own at the next check.");
  expect(statusDescription("pending", { garminMinutes: 20, stravaMinutes: 1 })).toBe(
    "Publishes to Strava on its own, usually within a minute.",
  );
  expect(statusDescription("pending", { garminMinutes: 20, stravaMinutes: 10 })).toBe(
    "Publishes to Strava on its own, usually within 10 min.",
  );
});

test("the detail line prefers the hold reason over the generic held description", () => {
  expect(activityExplanation({ publishStatus: "held", holdReason: "backlog" })).toBe(
    "Older activity found after a reconnect",
  );
  expect(activityExplanation({ publishStatus: "excluded", holdReason: null })).toBe(
    "Kept off Strava for good, unless you restore it.",
  );
});

test("list rows only explain the surprising waits", () => {
  expect(rowReason({ publishStatus: "held", holdReason: "category" })).toBeNull();
  expect(rowReason({ publishStatus: "held", holdReason: null })).toBeNull();
  expect(rowReason({ publishStatus: "held", holdReason: "backlog" })).toBe(
    "Older activity found after a reconnect",
  );
  expect(rowReason({ publishStatus: "missing", holdReason: null })).toBe(
    "Removed from Strava after publishing",
  );
  expect(rowReason({ publishStatus: "pending", holdReason: null })).toBeNull();
});
