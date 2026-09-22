import { useState } from "react";
import { Link } from "react-router";

import { HevyWorkoutDetailContent, formatWorkoutTime } from "@/components/hevy-workout-detail";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ResponsiveOverlay } from "@/components/ui/responsive-overlay";
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
export type BackfillRowKind = "locked" | "tracked" | "match" | "create";

export function backfillRowKind(item: BackfillItem): BackfillRowKind {
  if (item.action === "needs_mapping") {
    return "locked";
  }
  if (item.action === "already_tracked") {
    return "tracked";
  }
  return item.twinActivityId != null ? "match" : "create";
}

/** The selectable subset — rows that are neither locked nor already tracked. Exported so the
 * route can derive "select all importable" and the footer count from the
 * exact same rule the row itself uses to decide whether its checkbox is
 * disabled, instead of two definitions drifting apart. */
export function isSelectable(item: BackfillItem): boolean {
  return item.action !== "needs_mapping" && item.action !== "already_tracked";
}

function formatBackfillDate(iso: string): string {
  return formatWorkoutTime(iso);
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
  if (kind === "match") {
    return `${date} → matches Garmin activity #${item.twinActivityId}`;
  }
  if (kind === "tracked") {
    return `${date} → already tracked in ActivSync`;
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
  const [previewOpen, setPreviewOpen] = useState(false);
  const kind = backfillRowKind(item);
  const locked = kind === "locked";
  const tracked = kind === "tracked";
  // An overlay over the hub, where the backfill scan lives inline — so
  // opening the editor never unmounts it and the preview and selection
  // survive the round trip.
  const mappingHref =
    locked && item.missingTemplateIds.length > 0
      ? `/hevy/mapping/${encodeURIComponent(item.missingTemplateIds[0])}`
      : null;

  return (
    <>
      {/* Two columns on mobile (checkbox | content), with the actions dropping
          to their own row underneath; three on sm+, actions back on the right.
          A flex row can't do that without the action group either overflowing
          (shrink-0) or squashing the title (shrink). */}
      <li
        className={cn(
          "grid grid-cols-[auto_minmax(0,1fr)] items-start gap-x-3.5 gap-y-3 border-b border-border/60 px-5 py-3 last:border-b-0 sm:grid-cols-[auto_minmax(0,1fr)_auto] sm:items-center md:px-6",
          // Row states are relative tints, not surfaces — they have to read
          // the same whether the list sits on a card or a nested panel.
          locked && "bg-warning/[0.04]",
          tracked && "bg-foreground/[0.03]",
        )}
      >
        <Checkbox
        className="mt-0.5 sm:mt-0"
        aria-label={
          locked
            ? `${item.title} needs mapping`
            : tracked
              ? `${item.title} already tracked`
              : `Select ${item.title}`
        }
        checked={selected}
        disabled={locked || tracked}
        onCheckedChange={locked || tracked ? undefined : onToggle}
        />
        <div className="min-w-0">
        <p
          className={cn(
            "truncate text-sm font-semibold",
            locked && "text-warning/80",
            tracked && "text-muted-foreground",
          )}
        >
          {item.title}
        </p>
        {/* Wraps on mobile: "→ matches Garmin activity #…" is the whole point
            of the row, and truncating it there hid exactly that. */}
        <p
          className={cn(
            "font-mono text-[11.5px] break-words sm:truncate",
            locked ? "text-warning/70" : "text-muted-foreground",
          )}
        >
          {describeRow(item, kind)}
        </p>
        </div>
        <div className="col-start-2 flex flex-wrap items-center gap-2 sm:col-start-3 sm:row-start-1 sm:justify-end">
          <Button
            type="button"
            variant="outline"
            size="sm"
            aria-label={`Preview ${item.title}`}
            onClick={() => setPreviewOpen(true)}
          >
            Preview
          </Button>
          {item.garminUrl ? (
            <Button asChild variant="outline" size="sm">
              <a href={item.garminUrl} target="_blank" rel="noreferrer">
                Garmin ↗
              </a>
            </Button>
          ) : null}
          {item.stravaUrl ? (
            <Button asChild variant="outline" size="sm">
              <a href={item.stravaUrl} target="_blank" rel="noreferrer">
                Strava ↗
              </a>
            </Button>
          ) : null}
        {kind === "match" ? (
        <span className="shrink-0 rounded-md bg-success/10 px-2.5 py-1 font-mono text-[10.5px] font-semibold text-success uppercase">
          Match
        </span>
        ) : null}
        {item.stravaActivityId ? (
          <span className="shrink-0 rounded-md bg-[#fc4c02]/10 px-2.5 py-1 font-mono text-[10.5px] font-semibold text-[#d94500] uppercase">
            On Strava
          </span>
        ) : null}
        {kind === "create" ? (
        <span className="shrink-0 rounded-md bg-primary/10 px-2.5 py-1 font-mono text-[10.5px] font-semibold text-primary uppercase">
          Create
        </span>
        ) : null}
        {tracked ? (
        <span className="shrink-0 rounded-md bg-secondary px-2.5 py-1 font-mono text-[10.5px] font-semibold text-muted-foreground uppercase">
          Tracked
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
        </div>
      </li>
      <ResponsiveOverlay
        open={previewOpen}
        onOpenChange={setPreviewOpen}
        title={item.title}
        description="Workout data recorded by Hevy"
        mobile="sheet"
        size="wide"
      >
        <HevyWorkoutDetailContent detail={item.workout} />
      </ResponsiveOverlay>
    </>
  );
}
