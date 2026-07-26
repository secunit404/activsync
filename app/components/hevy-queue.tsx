import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { HevyWorkoutDetailContent } from "@/components/hevy-workout-detail";
import { ResponsiveOverlay } from "@/components/ui/responsive-overlay";
import { Spinner } from "@/components/ui/spinner";
import {
  chooseHevyMatch,
  getHevyWorkout,
  runHevyQueueAction,
  type HevyMatchStrategy,
  type HevyQueueAction,
  type HevyQueueItem,
  type HevyQueueState,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { ERROR_TOAST_DURATION_MS } from "@/lib/toast-duration";
import { cn } from "@/lib/utils";

type QueueCommand = HevyQueueAction | HevyMatchStrategy;
type ConfirmableAction = "skip" | "resync-fresh" | "replace";

const CONFIRM_COPY: Record<
  ConfirmableAction,
  { title: string; describe: (title: string) => string; confirmLabel: string; pendingLabel: string }
> = {
  skip: {
    title: "Skip this workout?",
    describe: (title) => `Skip ${title}?`,
    confirmLabel: "Skip",
    pendingLabel: "Skipping…",
  },
  "resync-fresh": {
    title: "Re-sync this workout?",
    describe: (title) => `Re-sync ${title} as a fresh Garmin upload?`,
    confirmLabel: "Re-sync fresh",
    pendingLabel: "Queuing…",
  },
  replace: {
    title: "Replace this Garmin workout?",
    describe: (title) =>
      `Replace the matched Garmin workout with ${title}? The original is preserved in ActivSync's recovery journal.`,
    confirmLabel: "Replace Garmin workout",
    pendingLabel: "Replacing…",
  },
};

function isMatchStrategy(action: QueueCommand): action is HevyMatchStrategy {
  return action === "merge" || action === "replace" || action === "describe";
}

type QueueRowKind = "in-flight" | "problem" | "skipped";
type Tone = "info" | "warning" | "destructive" | "muted";

const toneClasses: Record<Tone, { dot: string; badge: string }> = {
  info: { dot: "bg-info", badge: "bg-info/12 text-info" },
  warning: { dot: "bg-warning", badge: "bg-warning/12 text-warning" },
  destructive: { dot: "bg-destructive", badge: "bg-destructive/12 text-destructive" },
  muted: { dot: "bg-muted-foreground/50", badge: "bg-secondary text-muted-foreground" },
};

/**
 * Frame `3a`'s "Sync queue" card. Restyled from the Task-5 scaffold (which
 * grouped rows under "Needs attention"/"In flight"/"Skipped" subheadings) to
 * the handoff's flat, dot-indicated list — the count badges in the header
 * already say what each row is. Orphaned since Task 5 (nothing imported it);
 * this is the first task to wire it into a route.
 */
export function HevyQueue({
  state,
  onQueueChanged,
}: {
  state: HevyQueueState;
  onQueueChanged?: () => void;
}) {
  const queryClient = useQueryClient();
  const [confirmTarget, setConfirmTarget] = useState<
    { item: HevyQueueItem; action: ConfirmableAction } | null
  >(null);
  const [detailTarget, setDetailTarget] = useState<HevyQueueItem | null>(null);
  const action = useMutation({
    mutationFn: ({ hevyId, action }: { hevyId: string; action: QueueCommand }) =>
      isMatchStrategy(action)
        ? chooseHevyMatch(hevyId, action)
        : runHevyQueueAction(hevyId, action),
    onSuccess: (result) => {
      toast.success(result.message);
      onQueueChanged?.();
    },
    onError: (error) => toast.error(error.message, { duration: ERROR_TOAST_DURATION_MS }),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue }),
        queryClient.invalidateQueries({ queryKey: queryKeys.appState }),
        queryClient.invalidateQueries({ queryKey: queryKeys.allActivities }),
      ]);
    },
  });

  const runAction = (item: HevyQueueItem, nextAction: QueueCommand) => {
    if (
      nextAction === "skip" ||
      nextAction === "resync-fresh" ||
      nextAction === "replace"
    ) {
      setConfirmTarget({ item, action: nextAction });
      return;
    }
    action.mutate({ hevyId: item.hevyId, action: nextAction });
  };

  const confirmPending =
    confirmTarget !== null &&
    action.isPending &&
    action.variables?.hevyId === confirmTarget.item.hevyId;

  const rows: Array<{ item: HevyQueueItem; kind: QueueRowKind }> = [
    ...state.inFlight.map((item) => ({ item, kind: "in-flight" as const })),
    ...state.problems.map((item) => ({ item, kind: "problem" as const })),
    ...state.skipped.map((item) => ({ item, kind: "skipped" as const })),
  ];

  return (
    <Card className="gap-0 py-0" aria-labelledby="hevy-queue-title">
      <CardHeader className="border-b border-border/70 py-4">
        <CardTitle>
          <h2 id="hevy-queue-title" className="text-[15px] font-bold">
            Sync queue
          </h2>
        </CardTitle>
      </CardHeader>
      <CardContent className="p-0">
        {rows.length === 0 ? (
          <p className="px-5 py-6 text-sm text-muted-foreground">
            Nothing needs attention. New Hevy matches will appear here for review.
          </p>
        ) : (
          <ul>
            {rows.map(({ item, kind }) => (
              <QueueRow
                key={item.hevyId}
                item={item}
                kind={kind}
                busy={action.isPending}
                pending={action.isPending && action.variables?.hevyId === item.hevyId}
                onView={() => setDetailTarget(item)}
                onAction={(nextAction) => runAction(item, nextAction)}
              />
            ))}
          </ul>
        )}
      </CardContent>
      {confirmTarget && (
        <ConfirmDialog
          open
          onOpenChange={(next) => {
            if (!next) setConfirmTarget(null);
          }}
          title={CONFIRM_COPY[confirmTarget.action].title}
          description={CONFIRM_COPY[confirmTarget.action].describe(confirmTarget.item.title)}
          confirmLabel={CONFIRM_COPY[confirmTarget.action].confirmLabel}
          pendingLabel={CONFIRM_COPY[confirmTarget.action].pendingLabel}
          pending={confirmPending}
          onConfirm={async () => {
            await action.mutateAsync({
              hevyId: confirmTarget.item.hevyId,
              action: confirmTarget.action,
            });
            setConfirmTarget(null);
          }}
        />
      )}
      {detailTarget && (
        <WorkoutDetails
          item={detailTarget}
          onClose={() => setDetailTarget(null)}
        />
      )}
    </Card>
  );
}

function toneFor(kind: QueueRowKind, item: HevyQueueItem): Tone {
  if (kind === "in-flight") return "info";
  if (kind === "skipped") return "muted";
  return item.status === "failed" ? "destructive" : "warning";
}

const QUEUE_STATUS_LABELS: Record<string, string> = {
  waiting_watch: "Waiting for Garmin",
  awaiting_match: "Needs review",
  syncing: "Syncing",
  needs_mapping: "Needs mapping",
  failed: "Failed",
  needs_review: "Needs review",
  skipped: "Skipped",
};

function queueStatusLabel(item: HevyQueueItem): string {
  if (
    item.status === "waiting_watch" &&
    item.matchedGarminActivityId !== null
  ) {
    return "Queued";
  }
  return QUEUE_STATUS_LABELS[item.status] ?? item.status.replaceAll("_", " ");
}

function QueueRow({
  item,
  kind,
  busy,
  pending,
  onView,
  onAction,
}: {
  item: HevyQueueItem;
  kind: QueueRowKind;
  busy: boolean;
  pending: boolean;
  onView: () => void;
  onAction: (action: QueueCommand) => void;
}) {
  const tone = toneFor(kind, item);

  return (
    // Two columns on mobile (dot | content) with the actions on their own row
    // underneath, three on sm+. A wrapping flex row right-justified the
    // buttons into a ragged stair-step; a grid gives them one left edge.
    <li
      className={cn(
        "grid grid-cols-[auto_minmax(0,1fr)] items-start gap-x-3.5 gap-y-3 border-b border-border/50 px-5 py-3.5 last:border-b-0 sm:grid-cols-[auto_minmax(0,1fr)_auto] sm:items-center",
        // Only the problem tint survives. `awaitingMatch` also carried a
        // `bg-info/[0.04]` wash, but the surfaces are themselves blue-grey
        // (oklch hue 254), so an info blue at 4% never read as a highlight —
        // it read as a colour cast on the row. The dot and the badge already
        // say the row is awaiting a match. Warning stays because orange is
        // far enough off the surface hue to actually register as an alert.
        kind === "problem" && "bg-warning/[0.04]",
      )}
    >
      <span
        className={cn("mt-1.5 size-2 shrink-0 rounded-full sm:mt-0", toneClasses[tone].dot)}
        aria-hidden="true"
      />
      <div className="min-w-0">
        <p
          className={cn(
            "truncate text-[14.5px] font-semibold",
            kind === "skipped" && "text-muted-foreground",
          )}
        >
          {item.title}
        </p>
        {/* Wraps on mobile: which Garmin activity it matched is the reason
            the row is asking for a decision. */}
        <p
          className={cn(
            "font-mono text-xs break-words sm:truncate",
            kind === "problem" ? "text-warning/90" : "text-muted-foreground",
          )}
        >
          {item.startDisplay} ·{" "}
          {item.awaitingMatch
            ? `Matched ${item.matchedGarminTitle ?? `Garmin activity ${item.matchedGarminActivityId}`}${item.matchedStravaActivityId ? " · Already on Strava" : ""}`
            : (item.error ?? queueStatusLabel(item))}
        </p>
      </div>
      <div
        className="col-start-2 flex flex-wrap items-center gap-2 sm:col-start-3 sm:row-start-1 sm:justify-end"
        aria-busy={pending}
      >
        <ActionButton
          label="View workout"
          busyLabel="Opening…"
          disabled={false}
          pending={false}
          onClick={onView}
        />
        {item.matchedStravaUrl ? (
          <Button asChild variant="outline" size="sm">
            <a href={item.matchedStravaUrl} target="_blank" rel="noreferrer">
              Strava ↗
            </a>
          </Button>
        ) : null}
        {item.awaitingMatch ? (
          <>
            <ActionButton
              label="Merge"
              busyLabel="Applying…"
              disabled={busy}
              pending={pending}
              onClick={() => onAction("merge")}
            />
            {item.matchedStravaActivityId === null ? (
              <ActionButton
                label="Replace"
                busyLabel="Replacing…"
                disabled={busy}
                pending={pending}
                onClick={() => onAction("replace")}
              />
            ) : null}
            <ActionButton
              label="Description only"
              busyLabel="Applying…"
              disabled={busy}
              pending={pending}
              onClick={() => onAction("describe")}
            />
            <ActionButton
              label="Skip"
              busyLabel="Skipping…"
              disabled={busy}
              pending={pending}
              onClick={() => onAction("skip")}
            />
          </>
        ) : kind === "in-flight" ? (
          <span
            className={cn(
              "rounded-md px-2.5 py-1 font-mono text-[11px] whitespace-nowrap",
              toneClasses.info.badge,
            )}
          >
            {queueStatusLabel(item).toUpperCase()}
          </span>
        ) : null}
        {kind === "problem" ? (
          <>
            {item.needsMapping ? (
              // Not a link: `HevyQueueItem` has no `templateId`, so this
              // can't deep-link to a specific mapping editor entry the way
              // the Exercise mapping card's rows can — that card is the
              // actionable surface for mapping. This is informational only,
              // pointing at where to go without duplicating that link (and
              // without an ambiguous second "map" accessible name on the
              // page).
              <span className="rounded-md bg-warning/12 px-2.5 py-1 font-mono text-[11px] text-warning uppercase">
                Needs mapping — see below
              </span>
            ) : null}
            {item.resyncable ? (
              <ActionButton
                label="Re-sync fresh"
                busyLabel="Queuing…"
                disabled={busy}
                pending={pending}
                onClick={() => onAction("resync-fresh")}
              />
            ) : null}
            <ActionButton
              label="Retry"
              busyLabel="Queuing…"
              disabled={busy}
              pending={pending}
              onClick={() => onAction("retry")}
            />
            {!item.hasOpenOperation ? (
              <ActionButton
                label="Skip"
                busyLabel="Skipping…"
                disabled={busy}
                pending={pending}
                onClick={() => onAction("skip")}
              />
            ) : null}
          </>
        ) : null}
        {kind === "skipped" ? (
          <ActionButton
            label="Unskip"
            busyLabel="Restoring…"
            disabled={busy}
            pending={pending}
            onClick={() => onAction("unskip")}
          />
        ) : null}
      </div>
    </li>
  );
}

function WorkoutDetails({ item, onClose }: { item: HevyQueueItem; onClose: () => void }) {
  const detail = useQuery({
    queryKey: queryKeys.hevyWorkout(item.hevyId),
    queryFn: ({ signal }) => getHevyWorkout(item.hevyId, signal),
  });

  return (
    <ResponsiveOverlay
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={item.title}
      description="Workout data recorded by Hevy"
      mobile="sheet"
      size="wide"
    >
      {detail.isPending ? (
        <div className="flex min-h-40 items-center justify-center" aria-label="Loading workout">
          <Spinner />
        </div>
      ) : detail.isError ? (
        <p className="text-sm text-destructive">{detail.error.message}</p>
      ) : (
        <HevyWorkoutDetailContent detail={detail.data} />
      )}
    </ResponsiveOverlay>
  );
}

function ActionButton({
  label,
  busyLabel,
  disabled,
  pending,
  onClick,
}: {
  label: string;
  busyLabel: string;
  disabled: boolean;
  pending: boolean;
  onClick: () => void;
}) {
  return (
    <Button variant="outline" size="sm" disabled={disabled} onClick={onClick}>
      {pending ? <Spinner data-icon="inline-start" /> : null}
      {pending ? busyLabel : label}
    </Button>
  );
}
