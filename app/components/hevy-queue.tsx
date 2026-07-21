import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Spinner } from "@/components/ui/spinner";
import {
  runHevyQueueAction,
  type HevyQueueAction,
  type HevyQueueItem,
  type HevyQueueState,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";

/** The two queue actions that ask for confirmation before running. */
type ConfirmableAction = "skip" | "resync-fresh";

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
};

type QueueRowKind = "in-flight" | "problem" | "skipped";
type Tone = "info" | "warning" | "destructive" | "muted";

const toneClasses: Record<Tone, { dot: string; badge: string }> = {
  info: { dot: "bg-info", badge: "bg-info/12 text-info" },
  warning: { dot: "bg-warning", badge: "bg-warning/12 text-warning" },
  destructive: { dot: "bg-destructive", badge: "bg-destructive/12 text-destructive" },
  muted: { dot: "bg-muted-foreground/50", badge: "bg-muted text-muted-foreground" },
};

/**
 * Frame `3a`'s "Sync queue" card. Restyled from the Task-5 scaffold (which
 * grouped rows under "Needs attention"/"In flight"/"Skipped" subheadings) to
 * the handoff's flat, dot-indicated list — the count badges in the header
 * already say what each row is. Orphaned since Task 5 (nothing imported it);
 * this is the first task to wire it into a route.
 */
export function HevyQueue({ state }: { state: HevyQueueState }) {
  const queryClient = useQueryClient();
  const [confirmTarget, setConfirmTarget] = useState<
    { item: HevyQueueItem; action: ConfirmableAction } | null
  >(null);
  const action = useMutation({
    mutationFn: ({ hevyId, action }: { hevyId: string; action: HevyQueueAction }) =>
      runHevyQueueAction(hevyId, action),
    onSuccess: (result) => toast.success(result.message),
    onError: (error) => toast.error(error.message),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue }),
        queryClient.invalidateQueries({ queryKey: queryKeys.appState }),
        queryClient.invalidateQueries({ queryKey: queryKeys.allActivities }),
      ]);
    },
  });

  const runAction = (item: HevyQueueItem, nextAction: HevyQueueAction) => {
    if (nextAction === "skip" || nextAction === "resync-fresh") {
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
        <CardTitle className="flex flex-wrap items-center gap-2.5">
          <h2 id="hevy-queue-title" className="text-[15px] font-bold">
            Sync queue
          </h2>
          <QueueCountBadge count={state.counts.inFlight} label="in flight" tone="info" />
          <QueueCountBadge count={state.counts.problems} label="problem" tone="warning" />
          <QueueCountBadge count={state.counts.skipped} label="skipped" tone="muted" />
        </CardTitle>
      </CardHeader>
      <CardContent className="p-0">
        {rows.length === 0 ? (
          <p className="px-5 py-6 text-sm text-muted-foreground">
            Nothing in the queue — Hevy workouts sync automatically as they come in.
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
    </Card>
  );
}

function QueueCountBadge({
  count,
  label,
  tone,
}: {
  count: number;
  label: string;
  tone: Tone;
}) {
  if (count === 0) {
    return null;
  }
  // Compact status-pill copy, not a sentence — the design handoff keeps
  // these singular regardless of count ("2 IN FLIGHT", "1 PROBLEM").
  return (
    <span
      className={cn(
        "rounded-md px-2 py-0.5 font-mono text-[11px] font-semibold tracking-[0.02em] whitespace-nowrap uppercase",
        toneClasses[tone].badge,
      )}
    >
      {count} {label}
    </span>
  );
}

function toneFor(kind: QueueRowKind, item: HevyQueueItem): Tone {
  if (kind === "in-flight") return "info";
  if (kind === "skipped") return "muted";
  return item.status === "failed" ? "destructive" : "warning";
}

function QueueRow({
  item,
  kind,
  busy,
  pending,
  onAction,
}: {
  item: HevyQueueItem;
  kind: QueueRowKind;
  busy: boolean;
  pending: boolean;
  onAction: (action: HevyQueueAction) => void;
}) {
  const tone = toneFor(kind, item);

  return (
    <li
      className={cn(
        "flex items-center gap-3.5 border-b border-border/50 px-5 py-3.5 last:border-b-0",
        kind === "problem" && "bg-warning/[0.04]",
      )}
    >
      <span
        className={cn("size-2 shrink-0 rounded-full", toneClasses[tone].dot)}
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <p
          className={cn(
            "truncate text-[14.5px] font-semibold",
            kind === "skipped" && "text-muted-foreground",
          )}
        >
          {item.title}
        </p>
        <p
          className={cn(
            "truncate font-mono text-xs",
            kind === "problem" ? "text-warning/90" : "text-muted-foreground",
          )}
        >
          {item.startDisplay} · {item.error ?? item.status.replaceAll("_", " ")}
        </p>
      </div>
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-2" aria-busy={pending}>
        {kind === "in-flight" ? (
          <span
            className={cn(
              "rounded-md px-2.5 py-1 font-mono text-[11px] whitespace-nowrap",
              toneClasses.info.badge,
            )}
          >
            {item.status.replaceAll("_", " ").toUpperCase()}
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
