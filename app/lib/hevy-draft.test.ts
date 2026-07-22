import { expect, test } from "vitest";

import {
  hevyDraftFromState,
  hevyDraftsEqual,
  hevyDraftToPayload,
} from "./hevy-draft";
import type { SettingsState } from "./api";

const hevy: SettingsState["hevy"] = {
  connected: true,
  status: "Connected",
  apiKeySaved: true,
  enabled: true,
  watchStrategy: "replace",
  matchMode: "automatic",
  descriptionTemplate: "{title}\n{exercises}",
  summaryOnStructured: true,
  graceMinutes: 20,
  pollIntervalMinutes: 10,
  identity: { manufacturer: null, product: null, serial: null },
  identityDisplay: "not yet detected",
  profileOverride: { weightKg: null, birthYear: null, vo2max: null, sex: null },
};

test("round-trips a state with no overrides through draft and back to a payload", () => {
  const draft = hevyDraftFromState(hevy);
  expect(draft.identity).toEqual({ manufacturer: "", product: "", serial: "" });
  expect(draft.profile.sex).toBe("");

  const payload = hevyDraftToPayload(draft);
  expect(payload.descriptionTemplate).toBe("{title}\n{exercises}");
  expect(payload.summaryOnStructured).toBe(true);
  expect(payload.identity).toEqual({ manufacturer: null, product: null, serial: null });
  expect(payload.profileOverride).toEqual({
    weightKg: null,
    birthYear: null,
    vo2max: null,
    sex: null,
  });
});

test("round-trips numeric overrides", () => {
  const draft = hevyDraftFromState({
    ...hevy,
    identity: { manufacturer: 1, product: 2, serial: 3 },
    profileOverride: { weightKg: 82.5, birthYear: 1990, vo2max: 48, sex: "male" },
  });
  const payload = hevyDraftToPayload(draft);
  expect(payload.identity).toEqual({ manufacturer: 1, product: 2, serial: 3 });
  expect(payload.profileOverride).toEqual({
    weightKg: 82.5,
    birthYear: 1990,
    vo2max: 48,
    sex: "male",
  });
});

test("draft equality is field-order independent structural equality", () => {
  const a = hevyDraftFromState(hevy);
  const b = hevyDraftFromState(hevy);
  expect(hevyDraftsEqual(a, b)).toBe(true);
  expect(hevyDraftsEqual(a, { ...b, enabled: !b.enabled })).toBe(false);
});
