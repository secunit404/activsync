export type DraftMerge<T> = { value: T; conflict: boolean };

/**
 * Three-way merge of an owner's draft against a refetched server value.
 * A leaf the owner has not changed (draft equals the previous server value)
 * follows the new server value; a leaf they have changed keeps their draft,
 * and is reported as a conflict when the server moved it to something else.
 */
export function mergeDraft<T>(draft: T, previous: T, next: T): DraftMerge<T> {
  if (draft instanceof Set && previous instanceof Set && next instanceof Set) {
    return mergeSets(draft, previous, next) as DraftMerge<T>;
  }
  if (isRecord(draft) && isRecord(previous) && isRecord(next)) {
    let conflict = false;
    const value: Record<string, unknown> = {};
    for (const key of Object.keys(draft)) {
      const merged = mergeDraft(draft[key], previous[key], next[key]);
      value[key] = merged.value;
      conflict = conflict || merged.conflict;
    }
    return { value: value as T, conflict };
  }
  if (Object.is(draft, previous)) return { value: next, conflict: false };
  return {
    value: draft,
    conflict: !Object.is(next, previous) && !Object.is(next, draft),
  };
}

function mergeSets(
  draft: Set<string>,
  previous: Set<string>,
  next: Set<string>,
): DraftMerge<Set<string>> {
  const value = new Set<string>();
  let conflict = false;
  for (const key of new Set([...draft, ...previous, ...next])) {
    const merged = mergeDraft(draft.has(key), previous.has(key), next.has(key));
    if (merged.value) value.add(key);
    conflict = conflict || merged.conflict;
  }
  return { value, conflict };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
