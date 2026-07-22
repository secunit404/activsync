import { Link } from "react-router";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import type { BackfillResult } from "@/lib/api";
import { cn } from "@/lib/utils";

export type BackfillItem = BackfillResult["items"][number];

/**
 * A row's presentation kind — derived per item, never from a hardcoded
 * switch over `action`. `hevy_backfill.py` has more action values than
 * "locked or not" (`already_tracked`, `needs_review`, `linked_existing`,
 * `waiting_watch`, `passive`, plus the three watch strategies), so the only
 * reliable signal for "will this attach to an existing Garmin activity or
 * create a new one" is whether the item carries a `twinActivityId` — not
 * which specific action string it happens to be.
 */
export type BackfillRowKind = "locked" | "link" | "create";

export function backfillRowKind(item: BackfillItem): BackfillRowKind {
  if (item.action === "needs_mapping") {
    return "locked";
  }
  return item.twinActivityId != null ? "link" : "create";
}

/** The selectable subset — every row except locked ones. Exported so the
 * route can derive "select all importable" and the footer count from the
 * exact same rule the row itself uses to decide whether its checkbox is
 * disabled, instead of two definitions drifting apart. */
export function isSelectable(item: BackfillItem): boolean {
  return item.action !== "needs_mapping";
}

function formatBackfillDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" }).toUpperCase();
}

function humanizeAction(action: string): string {
  return action.replaceAll("_", " ");
}

/** The subtitle line under the title — what will actually happen, or why it
 * can't yet. Deliberately never claims "will create" for an item that isn't
 * actually going to (e.g. `already_tracked`, which `run_items` skips as a
 * no-op) — it names the real action instead. */
function describeRow(item: BackfillItem, kind: BackfillRowKind): string {
  const date = formatBackfillDate(item.startTime);
  if (kind === "link") {
    return `${date} → matches Garmin activity #${item.twinActivityId}`;
  }
  if (kind === "create") {
    return `${date} → ${humanizeAction(item.action)}`;
  }
  // locked
  const missing = item.missingTemplateIds.length;
  if (missing > 0) {
    return `${date} → ${missing} exercise${missing === 1 ? "" : "s"} need${missing === 1 ? "s" : ""} mapping before this can import`;
  }
  // The Task 4 edge case: needs_mapping with an empty missingTemplateIds —
  // an exercise failed to resolve but reported no template id to link to
  // (Hevy sent none). There is nowhere for "Map →" to go, so the copy says
  // so instead of implying a fix is one click away.
  return `${date} → an exercise needs mapping, but has no linkable template`;
}

export function BackfillRow({
  item,
  selected,
  onToggle,
}: {
  item: BackfillItem;
  selected: boolean;
  onToggle: () => void;
}) {
  const kind = backfillRowKind(item);
  const locked = kind === "locked";
  // An overlay over the hub, where the backfill scan lives inline — so
  // opening the editor never unmounts it and the preview and selection
  // survive the round trip.
  const mappingHref =
    locked && item.missingTemplateIds.length > 0
      ? `/hevy/mapping/${encodeURIComponent(item.missingTemplateIds[0])}`
      : null;

  return (
    <li
      className={cn(
        "flex items-center gap-3.5 border-b border-border/60 px-5 py-3 last:border-b-0 md:px-6",
        locked && "bg-warning/[0.04]",
      )}
    >
      <Checkbox
        aria-label={locked ? `${item.title} needs mapping` : `Select ${item.title}`}
        checked={selected}
        disabled={locked}
        onCheckedChange={locked ? undefined : onToggle}
      />
      <div className="min-w-0 flex-1">
        <p className={cn("truncate text-sm font-semibold", locked && "text-warning/80")}>
          {item.title}
        </p>
        <p
          className={cn(
            "truncate font-mono text-[11.5px]",
            locked ? "text-warning/70" : "text-muted-foreground",
          )}
        >
          {describeRow(item, kind)}
        </p>
      </div>
      {kind === "link" ? (
        <span className="shrink-0 rounded-md bg-success/10 px-2.5 py-1 font-mono text-[10.5px] font-semibold text-success uppercase">
          Link
        </span>
      ) : null}
      {kind === "create" ? (
        <span className="shrink-0 rounded-md bg-primary/10 px-2.5 py-1 font-mono text-[10.5px] font-semibold text-primary uppercase">
          Create
        </span>
      ) : null}
      {locked ? (
        // One control for both locked cases. The dead end (no linkable
        // template — Hevy reported none) is the same button disabled with an
        // explanation, not a second differently-coloured badge.
        mappingHref ? (
          <Button asChild variant="warning" size="sm" className="shrink-0">
            <Link to={mappingHref}>Map →</Link>
          </Button>
        ) : (
          <Button
            variant="warning"
            size="sm"
            className="shrink-0"
            disabled
            title="This exercise has no linkable template — it can't be fixed from here."
          >
            Map →
          </Button>
        )
      ) : null}
    </li>
  );
}
