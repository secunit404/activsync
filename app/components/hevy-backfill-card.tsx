import { useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  BackfillRow,
  backfillRowKind,
  isSelectable,
  type BackfillItem,
} from "@/components/backfill-row";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useSelection } from "@/hooks/use-selection";
import { previewHevyBackfill, runHevyBackfill, type BackfillResult } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { ERROR_TOAST_DURATION_MS } from "@/lib/toast-duration";
import { cn } from "@/lib/utils";

function defaultSince(): string {
  const date = new Date();
  date.setUTCDate(date.getUTCDate() - 30);
  return date.toISOString().slice(0, 10);
}

/**
 * Import historical Hevy workouts as Garmin activities, inline on the hub.
 *
 * This used to be a card holding a single button that opened `/hevy/backfill`
 * as an overlay — two surfaces for one task, where the first contributed
 * nothing but a link. The date, the scan and its results all live here now,
 * and the route is gone.
 *
 * Being part of the hub page also removes the state problem the overlay had:
 * mapping a locked row opens `/hevy/mapping/:templateId` over the hub, so
 * this component is never unmounted and the preview and selection survive on
 * their own.
 *
 * `previewHevyBackfill` takes only `since` — Preview always re-scans the full
 * since-derived window. `runHevyBackfill` additionally takes the ticked
 * selection (`hevyIds`): only those workouts are ingested. Locked
 * (`needs_mapping`) rows are excluded from `selection` at the source — see
 * `isSelectable` — but the server enforces the same rule independently, so a
 * locked id can never be imported even if one somehow reached this call.
 */
export function HevyBackfillCard({
  mappingSavedToken = 0,
}: {
  /**
   * Bumped by the hub whenever the mapping editor saves. A just-mapped row
   * must unlock in place rather than keep showing the scan's stale verdict,
   * and the scan is a mutation, so no query invalidation can refresh it.
   */
  mappingSavedToken?: number;
}) {
  const queryClient = useQueryClient();
  const [since, setSince] = useState(defaultSince());
  const selection = useSelection<string>();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [appliedResult, setAppliedResult] = useState<BackfillResult | null>(null);

  const preview = useMutation({
    mutationFn: () => previewHevyBackfill(since),
    onError: (error) => {
      toast.error(
        error instanceof Error ? error.message : "Could not preview the backfill.",
        { duration: ERROR_TOAST_DURATION_MS },
      );
    },
  });

  const run = useMutation({
    mutationFn: () => runHevyBackfill(since, Array.from(selection.selected)),
    onSuccess: (result) => {
      toast.success(result.message);
      setConfirmOpen(false);
      selection.clear();
      // The queue now owns imported rows. Re-scan so they disappear from
      // backfill immediately; a later queue Skip makes them eligible again.
      preview.mutate();
    },
    onError: (error) => {
      toast.error(
        error instanceof Error ? error.message : "Could not run the backfill.",
        { duration: ERROR_TOAST_DURATION_MS },
      );
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue }),
        queryClient.invalidateQueries({ queryKey: queryKeys.appState }),
        queryClient.invalidateQueries({ queryKey: queryKeys.allActivities }),
      ]);
    },
  });

  // Re-scan after a mapping is saved, but only if a scan is already on
  // screen — otherwise this would kick off a fetch the user never asked for.
  // An effect (not a render-time adjustment) because firing a mutation is a
  // side effect, which is exactly what effects are for; `preview` is a stable
  // mutation object, so the token is the only real dependency.
  const previewMutate = preview.mutate;
  const hasPreview = preview.data != null;
  useEffect(() => {
    if (mappingSavedToken > 0 && hasPreview) {
      previewMutate();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mappingSavedToken]);

  // A fresh preview result must start with nothing selected — carrying a
  // selection across previews would risk importing rows the user never
  // actually saw in this run. Render-time state adjustment (not a
  // `useEffect`; this repo's ESLint enforces `react-hooks/set-state-in-effect`).
  // Keyed off object identity, not `since`: clicking Preview again for the
  // *same* since date still returns a new response object and must still
  // reset — the user is looking at a new snapshot, not the old one.
  const items: BackfillItem[] = (preview.data?.items ?? []).filter(
    (item) => item.action !== "already_tracked",
  );
  if (preview.data && preview.data !== appliedResult) {
    setAppliedResult(preview.data);
    selection.clear();
  }

  // The selectable set excludes locked and already-tracked rows —
  // this is the one true definition "select all importable" and the footer
  // count both read from. Never `items.length`.
  const selectableIds = items.filter(isSelectable).map((item) => item.hevyId);
  const allSelected =
    selectableIds.length > 0 && selectableIds.every((id) => selection.selected.has(id));
  const someSelected = selection.count > 0 && !allSelected;

  const counts = items.reduce(
    (acc, item) => {
      const kind = backfillRowKind(item);
      acc[kind] += 1;
      return acc;
    },
    { locked: 0, tracked: 0, match: 0, create: 0 },
  );

  return (
    <Card aria-labelledby="hevy-backfill-title" className="gap-0 py-0">
      <CardHeader className="border-b border-border/70 py-4">
        <CardTitle>
          <h2 id="hevy-backfill-title" className="text-[15px] font-bold">
            Backfill older workouts
          </h2>
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-5 py-5">
        <p className="text-sm text-muted-foreground">
          Scan Hevy workouts since a date and match them to Garmin. Preview
          first — nothing is written until you run it.
        </p>

        <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
          <Field className="sm:max-w-56 sm:flex-1">
            <FieldLabel
              htmlFor="backfill-since"
              className="font-mono text-xs tracking-[0.06em] text-muted-foreground uppercase"
            >
              Since
            </FieldLabel>
            <Input
              id="backfill-since"
              type="date"
              className="h-11"
              value={since}
              onChange={(event) => setSince(event.target.value)}
            />
          </Field>
          <Button
            type="button"
            variant="outline"
            size="xl"
            disabled={since.trim() === "" || preview.isPending}
            onClick={() => preview.mutate()}
          >
            {preview.isPending ? <Spinner data-icon="inline-start" /> : null}
            {preview.isPending ? "Scanning…" : "Preview"}
          </Button>
        </div>

        {preview.data ? (
          <>
            <p className="text-sm text-muted-foreground">{preview.data.message}</p>

            {items.length > 0 ? (
              <div className="flex flex-col gap-3.5 rounded-xl border border-border">
                <div className="flex flex-wrap items-center gap-2.5 border-b border-border px-4 pt-3.5 pb-3.5">
                  <Checkbox
                    id="backfill-select-all"
                    aria-label="Select all importable"
                    checked={allSelected ? true : someSelected ? "indeterminate" : false}
                    disabled={selectableIds.length === 0}
                    onCheckedChange={() => selection.toggleAll(selectableIds)}
                  />
                  <label
                    htmlFor="backfill-select-all"
                    className="text-[13px] text-muted-foreground"
                  >
                    Select all importable{" "}
                    <span className="text-muted-foreground/60">
                      ({selectableIds.length} of {items.length})
                    </span>
                  </label>
                  <span
                    className="mx-1 hidden h-4.5 w-px bg-border sm:block"
                    aria-hidden="true"
                  />
                  <CountBadge
                    count={items.length}
                    label="scanned"
                    className="bg-muted text-muted-foreground"
                  />
                  <CountBadge
                    count={counts.match}
                    label="matched"
                    className="bg-success/10 text-success"
                  />
                  <CountBadge
                    count={counts.create}
                    label="created"
                    className="bg-primary/10 text-primary"
                  />
                  <CountBadge
                    count={counts.locked}
                    label="needs mapping"
                    className="bg-warning/12 text-warning"
                  />
                  <CountBadge
                    count={counts.tracked}
                    label="tracked"
                    className="bg-muted text-muted-foreground"
                  />
                </div>
                <ul className="max-h-[360px] overflow-y-auto">
                  {items.map((item) => (
                    <BackfillRow
                      key={item.hevyId}
                      item={item}
                      selected={selection.selected.has(item.hevyId)}
                      onToggle={() => selection.toggle(item.hevyId)}
                    />
                  ))}
                </ul>
                <div className="flex justify-end px-4 pb-4">
                  <Button
                    type="button"
                    className="w-full sm:w-auto"
                    disabled={selection.count === 0 || run.isPending}
                    onClick={() => setConfirmOpen(true)}
                  >
                    {run.isPending ? <Spinner data-icon="inline-start" /> : null}
                    {run.isPending
                      ? "Importing…"
                      : `Import ${selection.count} selected`}
                  </Button>
                </div>
              </div>
            ) : null}
          </>
        ) : (
          <p className="text-sm text-muted-foreground">
            {preview.isPending
              ? "Scanning Hevy for workouts…"
              : "Choose a start date and preview to see what would import."}
          </p>
        )}
      </CardContent>

      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title="Import these workouts?"
        description={`Import ${selection.count} historical Hevy workout${selection.count === 1 ? "" : "s"}? Matches go to your review queue; the rest are queued to sync in.`}
        confirmLabel="Import"
        pendingLabel="Importing…"
        pending={run.isPending}
        onConfirm={() => run.mutate()}
      />
    </Card>
  );
}

function CountBadge({
  count,
  label,
  className,
}: {
  count: number;
  label: string;
  className: string;
}) {
  if (count === 0) {
    return null;
  }
  return (
    <span
      className={cn(
        "rounded-md px-2 py-0.5 font-mono text-[11px] font-semibold whitespace-nowrap uppercase",
        className,
      )}
    >
      {count} {label}
    </span>
  );
}
