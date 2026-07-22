import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Outlet, useNavigate } from "react-router";
import { toast } from "sonner";

import {
  BackfillRow,
  backfillRowKind,
  isSelectable,
  type BackfillItem,
} from "@/components/backfill-row";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ResponsiveOverlay } from "@/components/ui/responsive-overlay";
import { Spinner } from "@/components/ui/spinner";
import { useSelection } from "@/hooks/use-selection";
import { previewHevyBackfill, runHevyBackfill, type BackfillResult } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { ERROR_TOAST_DURATION_MS } from "@/lib/toast-duration";
import { cn } from "@/lib/utils";

/**
 * Threaded to the nested mapping editor so a saved mapping can re-run the
 * preview — a just-mapped row must unlock in place rather than stay locked
 * against stale scan data.
 */
export type BackfillOutletContext = { onMappingSaved: () => void };

function defaultSince(): string {
  const date = new Date();
  date.setUTCDate(date.getUTCDate() - 30);
  return date.toISOString().slice(0, 10);
}

/**
 * Backfill (frame `4b`): import historical Hevy workouts as Garmin
 * activities. Nested under `/hevy` (Task 14's hub) as a sibling of
 * `/hevy/mapping/:templateId` (Task 15) — both mount through the hub's own
 * `<Outlet />`, so this issues no fetch of its own for connection state; it
 * only reads/writes through `previewHevyBackfill`/`runHevyBackfill`.
 *
 * `previewHevyBackfill` takes only `since` — Preview always re-scans the
 * full since-derived window. `runHevyBackfill` additionally takes the
 * ticked selection (`hevyIds`): only those workouts are ingested. Locked
 * (`needs_mapping`) rows are excluded from `selection` at the source — see
 * `isSelectable` — but the server enforces the same rule independently, so
 * a locked id can never be imported even if one somehow reached this call.
 */
export default function HevyBackfill() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [since, setSince] = useState(defaultSince());
  const selection = useSelection<string>();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [appliedResult, setAppliedResult] = useState<BackfillResult | null>(null);

  const close = () => navigate("..");
  const handleOpenChange = (open: boolean) => {
    if (!open) close();
  };

  const preview = useMutation({
    mutationFn: () => previewHevyBackfill(since),
    onError: (error) => {
      toast.error(error instanceof Error ? error.message : "Could not preview the backfill.", {
        duration: ERROR_TOAST_DURATION_MS,
      });
    },
  });

  const run = useMutation({
    mutationFn: () => runHevyBackfill(since, Array.from(selection.selected)),
    onSuccess: (result) => {
      toast.success(result.message);
      setConfirmOpen(false);
      selection.clear();
    },
    onError: (error) => {
      toast.error(error instanceof Error ? error.message : "Could not run the backfill.", {
        duration: ERROR_TOAST_DURATION_MS,
      });
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue }),
        queryClient.invalidateQueries({ queryKey: queryKeys.appState }),
        queryClient.invalidateQueries({ queryKey: queryKeys.allActivities }),
      ]);
    },
  });

  // A fresh preview result must start with nothing selected — carrying a
  // selection across previews would risk importing rows the user never
  // actually saw in this run. Render-time state adjustment (not a
  // `useEffect`; this repo's ESLint enforces `react-hooks/set-state-in-effect`),
  // same technique as hevy-mapping.tsx's `appliedTemplateId`. Keyed off
  // object identity, not `since`: clicking Preview again for the *same*
  // since date still returns a new response object and must still reset —
  // the user is looking at a new snapshot, not the old one.
  const items: BackfillItem[] = preview.data?.items ?? [];
  if (preview.data && preview.data !== appliedResult) {
    setAppliedResult(preview.data);
    selection.clear();
  }

  // The selectable set is everything except locked (`needs_mapping`) rows —
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
    { locked: 0, link: 0, create: 0 },
  );

  return (
    <>
      <ResponsiveOverlay
        open
        onOpenChange={handleOpenChange}
        title="Backfill older workouts"
        description="Scan Hevy workouts since a date and match them to Garmin. Preview first — nothing is written until you run it."
        mobile="sheet"
        size="wide"
        footer={
          preview.data ? (
            <Button
              type="button"
              className="w-full md:w-auto"
              disabled={selection.count === 0 || run.isPending}
              onClick={() => setConfirmOpen(true)}
            >
              {run.isPending ? <Spinner data-icon="inline-start" /> : null}
              {run.isPending ? "Importing…" : `Import ${selection.count} selected`}
            </Button>
          ) : undefined
        }
      >
        <div className="flex flex-col gap-5">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
            <Field className="flex-1">
              <FieldLabel
                htmlFor="backfill-since"
                className="font-mono text-xs tracking-[0.06em] text-muted-foreground uppercase"
              >
                Since
              </FieldLabel>
              <Input
                id="backfill-since"
                type="date"
                value={since}
                onChange={(event) => setSince(event.target.value)}
              />
            </Field>
            <Button
              type="button"
              variant="outline"
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
                    <label htmlFor="backfill-select-all" className="text-[13px] text-muted-foreground">
                      Select all importable{" "}
                      <span className="text-muted-foreground/60">
                        ({selectableIds.length} of {items.length})
                      </span>
                    </label>
                    <span className="mx-1 hidden h-4.5 w-px bg-border sm:block" aria-hidden="true" />
                    <CountBadge count={items.length} label="scanned" className="bg-muted text-muted-foreground" />
                    <CountBadge count={counts.link} label="linked" className="bg-success/10 text-success" />
                    <CountBadge count={counts.create} label="created" className="bg-primary/10 text-primary" />
                    <CountBadge
                      count={counts.locked}
                      label="needs mapping"
                      className="bg-warning/12 text-warning"
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
        </div>
      </ResponsiveOverlay>

      {/* The nested mapping editor (`/hevy/backfill/mapping/:templateId`)
          mounts here, on top of this overlay. This component stays mounted,
          so `preview.data` and `selection` survive the round trip. */}
      <Outlet context={{ onMappingSaved: () => preview.mutate() }} />

      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title="Import these workouts?"
        description={`Import ${selection.count} historical Hevy workout${selection.count === 1 ? "" : "s"}? Linked ones attach to their matching Garmin activity; the rest are queued to sync in.`}
        confirmLabel="Import"
        pendingLabel="Importing…"
        pending={run.isPending}
        onConfirm={() => run.mutate()}
      />
    </>
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
