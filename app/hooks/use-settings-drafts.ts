import { useState } from "react";

import type { SettingsState } from "@/lib/api";
import { hevyDraftFromState, hevyDraftsEqual, type HevyDraft } from "@/lib/hevy-draft";

type PreferencesDraft = SettingsState["preferences"];

/**
 * Owns the three dirty-trackable Settings drafts (preferences, the enabled
 * activity-type set, Hevy config) behind the single shared Discard/Save
 * footer. Connections isn't here — its dialogs act immediately and aren't
 * part of the pending-changes flow.
 *
 * Resyncing to fresh server data (e.g. after a successful Save invalidates
 * and refetches `settings`) happens by comparing `state` to the last-synced
 * reference during render and resetting drafts right there if it changed —
 * React's documented alternative to a `useEffect` for this exact case, and
 * why `state` must come from a query with structural sharing (react-query's
 * default) so the reference is only new when the data actually changed.
 */
export function useSettingsDrafts(state: SettingsState) {
  const [syncedState, setSyncedState] = useState(state);
  const [preferencesDraft, setPreferencesDraft] = useState(state.preferences);
  const [selectedTypes, setSelectedTypes] = useState(() =>
    enabledTypeKeys(state.activityTypes),
  );
  const [hevyDraft, setHevyDraft] = useState(() => hevyDraftFromState(state.hevy));
  const [discardToken, setDiscardToken] = useState(0);

  if (state !== syncedState) {
    setSyncedState(state);
    setPreferencesDraft(state.preferences);
    setSelectedTypes(enabledTypeKeys(state.activityTypes));
    setHevyDraft(hevyDraftFromState(state.hevy));
  }

  const preferencesDirty = !preferencesEqual(preferencesDraft, state.preferences);
  const activityTypesDirty = !setsEqual(
    selectedTypes,
    enabledTypeKeys(state.activityTypes),
  );
  const hevyDirty =
    state.hevy.connected && !hevyDraftsEqual(hevyDraft, hevyDraftFromState(state.hevy));

  function discard() {
    setPreferencesDraft(state.preferences);
    setSelectedTypes(enabledTypeKeys(state.activityTypes));
    setHevyDraft(hevyDraftFromState(state.hevy));
    setDiscardToken((token) => token + 1);
  }

  return {
    preferencesDraft,
    setPreferencesDraft,
    selectedTypes,
    setSelectedTypes,
    hevyDraft,
    setHevyDraft,
    preferencesDirty,
    activityTypesDirty,
    hevyDirty,
    dirty: preferencesDirty || activityTypesDirty || hevyDirty,
    discard,
    discardToken,
  };
}

export type HevyDraftValue = HevyDraft;

function enabledTypeKeys(
  activityTypes: SettingsState["activityTypes"],
): Set<string> {
  return new Set(
    activityTypes
      .filter((activityType) => activityType.autosync)
      .map((activityType) => activityType.typeKey),
  );
}

function preferencesEqual(a: PreferencesDraft, b: PreferencesDraft): boolean {
  return (
    a.displayTimezone === b.displayTimezone &&
    a.garminPollIntervalMinutes === b.garminPollIntervalMinutes &&
    a.stravaPollIntervalMinutes === b.stravaPollIntervalMinutes &&
    a.lookbackDays === b.lookbackDays
  );
}

function setsEqual(a: Set<string>, b: Set<string>): boolean {
  if (a.size !== b.size) return false;
  for (const value of a) {
    if (!b.has(value)) return false;
  }
  return true;
}
