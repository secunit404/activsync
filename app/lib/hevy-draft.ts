import type { SettingsState } from "@/lib/api";

/**
 * String-backed mirror of `SettingsState["hevy"]`'s editable fields, shaped
 * for controlled number inputs (which need a string value, including the
 * empty string for "no override"). Lives alongside the Settings route's
 * other drafts (preferences, activity types) for the shared Discard/Save
 * footer's dirty-tracking.
 */
export type HevyDraft = {
  enabled: boolean;
  watchStrategy: SettingsState["hevy"]["watchStrategy"];
  matchMode: SettingsState["hevy"]["matchMode"];
  descriptionTemplate: string;
  summaryOnStructured: boolean;
  graceMinutes: number;
  pollIntervalMinutes: number;
  identity: { manufacturer: string; product: string; serial: string };
  profile: {
    weightKg: string;
    birthYear: string;
    vo2max: string;
    sex: "" | "male" | "female";
  };
};

export function hevyDraftFromState(hevy: SettingsState["hevy"]): HevyDraft {
  return {
    enabled: hevy.enabled,
    watchStrategy: hevy.watchStrategy,
    matchMode: hevy.matchMode,
    descriptionTemplate: hevy.descriptionTemplate,
    summaryOnStructured: hevy.summaryOnStructured,
    graceMinutes: hevy.graceMinutes,
    pollIntervalMinutes: hevy.pollIntervalMinutes,
    identity: {
      manufacturer: nullableToString(hevy.identity.manufacturer),
      product: nullableToString(hevy.identity.product),
      serial: nullableToString(hevy.identity.serial),
    },
    profile: {
      weightKg: nullableToString(hevy.profileOverride.weightKg),
      birthYear: nullableToString(hevy.profileOverride.birthYear),
      vo2max: nullableToString(hevy.profileOverride.vo2max),
      sex: hevy.profileOverride.sex ?? "",
    },
  };
}

export function hevyDraftsEqual(a: HevyDraft, b: HevyDraft): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function hevyDraftToPayload(draft: HevyDraft) {
  return {
    enabled: draft.enabled,
    watchStrategy: draft.watchStrategy,
    matchMode: draft.matchMode,
    descriptionTemplate: draft.descriptionTemplate,
    summaryOnStructured: draft.summaryOnStructured,
    graceMinutes: draft.graceMinutes,
    pollIntervalMinutes: draft.pollIntervalMinutes,
    identity: {
      manufacturer: parseNullableNumber(draft.identity.manufacturer),
      product: parseNullableNumber(draft.identity.product),
      serial: parseNullableNumber(draft.identity.serial),
    },
    profileOverride: {
      weightKg: parseNullableNumber(draft.profile.weightKg),
      birthYear: parseNullableNumber(draft.profile.birthYear),
      vo2max: parseNullableNumber(draft.profile.vo2max),
      sex:
        draft.profile.sex === "male" || draft.profile.sex === "female"
          ? draft.profile.sex
          : null,
    },
  };
}

function nullableToString(value: number | null): string {
  return value === null ? "" : String(value);
}

function parseNullableNumber(value: string): number | null {
  return value.trim() ? Number(value) : null;
}
