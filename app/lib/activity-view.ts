import type { PublishStatus, StatusFilter } from "@/lib/api";

/** What the `status` URL param can hold: a filter, or an explicit "all". */
export type ActivityView = StatusFilter | "all";

const views = new Set<string>([
  "review",
  "all",
  "pending",
  "held",
  "published",
  "missing",
  "excluded",
] satisfies ActivityView[]);

/** Null when the param is absent or unknown — the caller picks the default. */
export function parseActivityView(raw: string | null): ActivityView | null {
  return raw !== null && views.has(raw) ? (raw as ActivityView) : null;
}

export function reviewCountOf(counts: Record<PublishStatus, number>): number {
  return counts.held + counts.missing;
}

/** Land on what needs a decision; with nothing waiting, show everything. */
export function defaultActivityView(reviewCount: number): ActivityView {
  return reviewCount > 0 ? "review" : "all";
}

export function statusFilterFor(view: ActivityView): StatusFilter | null {
  return view === "all" ? null : view;
}
