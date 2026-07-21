import { useCallback, useState } from "react";

export type UseSelectionResult<T> = {
  selected: ReadonlySet<T>;
  toggle: (id: T) => void;
  toggleAll: (ids: T[]) => void;
  clear: () => void;
  count: number;
};

/**
 * Bulk-selection state, generic over the id type — `T` defaults to `number`
 * so every pre-existing call site (`useSelection()` for numeric Garmin
 * activity ids) keeps its inferred type unchanged. Task 16 instantiates it
 * as `useSelection<string>()` for Hevy backfill rows, which are keyed by
 * `hevyId` (a string), not a numeric id.
 *
 * Every update returns a brand-new `Set` rather than mutating the previous
 * one — this is a hard project rule (see the "does not mutate the previous
 * set" test), not a stylistic preference, since callers may still be
 * holding a reference to the prior `selected` value (e.g. mid-render).
 */
export function useSelection<T = number>(): UseSelectionResult<T> {
  const [selected, setSelected] = useState<ReadonlySet<T>>(() => new Set());

  const toggle = useCallback((id: T) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) {
        next.add(id);
      }
      return next;
    });
  }, []);

  // A toggle, not a select-all: if every id passed in is already selected,
  // this clears the whole selection; otherwise it replaces the selection
  // with exactly `ids` (dropping anything selected outside that set, e.g.
  // from a page the caller no longer considers "current").
  const toggleAll = useCallback((ids: T[]) => {
    setSelected((prev) => (ids.every((id) => prev.has(id)) ? new Set() : new Set(ids)));
  }, []);

  const clear = useCallback(() => setSelected(new Set()), []);

  return { selected, toggle, toggleAll, clear, count: selected.size };
}
