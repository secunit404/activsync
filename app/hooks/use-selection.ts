import { useCallback, useState } from "react";

export type UseSelectionResult = {
  selected: ReadonlySet<number>;
  toggle: (id: number) => void;
  toggleAll: (ids: number[]) => void;
  clear: () => void;
  count: number;
};

/**
 * Bulk-selection state for the Activities table/cards. Every update returns
 * a brand-new `Set` rather than mutating the previous one — this is a hard
 * project rule (see the "does not mutate the previous set" test), not a
 * stylistic preference, since callers may still be holding a reference to
 * the prior `selected` value (e.g. mid-render).
 */
export function useSelection(): UseSelectionResult {
  const [selected, setSelected] = useState<ReadonlySet<number>>(() => new Set());

  const toggle = useCallback((id: number) => {
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
  const toggleAll = useCallback((ids: number[]) => {
    setSelected((prev) => (ids.every((id) => prev.has(id)) ? new Set() : new Set(ids)));
  }, []);

  const clear = useCallback(() => setSelected(new Set()), []);

  return { selected, toggle, toggleAll, clear, count: selected.size };
}
