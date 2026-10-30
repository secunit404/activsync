import { useState } from "react";

import type { SettingsState } from "@/lib/api";
import { mergeDraft } from "@/lib/draft-merge";
import { hevyDraftFromState, hevyDraftsEqual } from "@/lib/hevy-draft";

type PreferencesDraft = SettingsState["preferences"];

/**
 * Owns the three dirty-trackable Settings drafts (preferences, the enabled
 * activity-type set, Hevy config) behind the single shared Discard/Save
 * footer. Connections isn't here — its dialogs act immediately and aren't
 * part of the pending-changes flow.
 *
 * When `state` changes (SSE refresh, a connection action, a Save) the drafts
 * are merged during render rather than reset — React's documented alternative
 * to a `useEffect` for this case. Fields the owner changed keep their draft;
 * untouched fields follow the server. `serverChangedUnderDraft` flags a
 * touched field the server also moved. After a Save the draft equals the new
 * server value, so nothing stays dirty.
 */
export function useSettingsDrafts(state: SettingsState) {
  const [syncedState, setSyncedState] = useState(state);
  const [preferencesDraft, setPreferencesDraft] = useState(state.preferences);
  const [selectedTypes, setSelectedTypes] = useState(() =>
    enabledTypeKeys(state.activityTypes),
  );
  const [hevyDraft, setHevyDraft] = useState(() => hevyDraftFromState(state.hevy));
  const [discardToken, setDiscardToken] = useState(0);

  const [conflict, setConflict] = useState(false);

  if (state !== syncedState) {
    const preferences = mergeDraft(
      preferencesDraft,
      syncedState.preferences,
      state.preferences,
    );
    const types = mergeDraft(
      selectedTypes,
      enabledTypeKeys(syncedState.activityTypes),
      enabledTypeKeys(state.activityTypes),
    );
    const hevy = mergeDraft(
      hevyDraft,
      hevyDraftFromState(syncedState.hevy),
      hevyDraftFromState(state.hevy),
    );
    setSyncedState(state);
    setPreferencesDraft(preferences.value);
    setSelectedTypes(types.value);
    setHevyDraft(hevy.value);
    setConflict(preferences.conflict || types.conflict || hevy.conflict);
  }

  const preferencesDirty = !preferencesEqual(preferencesDraft, state.preferences);
  const activityTypesDirty = !setsEqual(
    selectedTypes,
    enabledTypeKeys(state.activityTypes),
  );
  const hevyDirty =
    state.hevy.connected && !hevyDraftsEqual(hevyDraft, hevyDraftFromState(state.hevy));

  const dirty = preferencesDirty || activityTypesDirty || hevyDirty;

  function discard() {
    setPreferencesDraft(state.preferences);
    setSelectedTypes(enabledTypeKeys(state.activityTypes));
    setHevyDraft(hevyDraftFromState(state.hevy));
    setDiscardToken((token) => token + 1);
    setConflict(false);
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
    dirty,
    serverChangedUnderDraft: dirty && conflict,
    discard,
    discardToken,
  };
}

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
