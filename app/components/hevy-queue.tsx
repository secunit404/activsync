import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import {
  runHevyQueueAction,
  type HevyQueueAction,
  type HevyQueueItem,
  type HevyQueueState,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function HevyQueue({ state }: { state: HevyQueueState }) {
  if (
    state.counts.inFlight === 0 &&
    state.counts.problems === 0 &&
    state.counts.skipped === 0
  ) {
    return null;
  }

  return <HevyQueueContent state={state} />;
}

function HevyQueueContent({ state }: { state: HevyQueueState }) {
  const queryClient = useQueryClient();
  const action = useMutation({
    mutationFn: ({
      hevyId,
      action,
    }: {
      hevyId: string;
      action: HevyQueueAction;
    }) => runHevyQueueAction(hevyId, action),
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
    if (
      (nextAction === "skip" || nextAction === "resync-fresh") &&
      !window.confirm(
        nextAction === "skip"
          ? `Skip ${item.title}?`
          : `Re-sync ${item.title} as a fresh Garmin upload?`,
      )
    ) {
      return;
    }
    action.mutate({ hevyId: item.hevyId, action: nextAction });
  };

  return (
    <Card aria-labelledby="hevy-queue-title">
      <CardHeader>
        <CardTitle>
          <h2 id="hevy-queue-title">Hevy sync</h2>
        </CardTitle>
        <CardDescription>
          Workouts currently being reconciled or waiting for your attention.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-6">
        {state.problems.length > 0 ? (
          <QueueSection title="Needs attention">
            {state.problems.map((item) => (
              <QueueRow
                key={item.hevyId}
                item={item}
                pending={action.isPending && action.variables?.hevyId === item.hevyId}
              >
                {item.needsMapping ? (
                  <Button asChild variant="outline" size="sm">
                    <Link to="/settings#hevy-mappings">Map exercises</Link>
                  </Button>
                ) : null}
                {item.resyncable ? (
                  <ActionButton
                    label="Re-sync fresh"
                    busy="Queuing…"
                    disabled={action.isPending}
                    pending={action.isPending && action.variables?.hevyId === item.hevyId}
                    onClick={() => runAction(item, "resync-fresh")}
                  />
                ) : null}
                <ActionButton
                  label="Retry"
                  busy="Queuing…"
                  disabled={action.isPending}
                  pending={action.isPending && action.variables?.hevyId === item.hevyId}
                  onClick={() => runAction(item, "retry")}
                />
                {!item.hasOpenOperation ? (
                  <ActionButton
                    label="Skip"
                    busy="Skipping…"
                    disabled={action.isPending}
                    pending={action.isPending && action.variables?.hevyId === item.hevyId}
                    onClick={() => runAction(item, "skip")}
                  />
                ) : null}
              </QueueRow>
            ))}
          </QueueSection>
        ) : null}

        {state.inFlight.length > 0 ? (
          <QueueSection title="In flight">
            {state.inFlight.map((item) => (
              <QueueRow key={item.hevyId} item={item} pending={false} />
            ))}
          </QueueSection>
        ) : null}

        {state.skipped.length > 0 ? (
          <QueueSection title={`Skipped (${state.skipped.length})`}>
            {state.skipped.map((item) => (
              <QueueRow
                key={item.hevyId}
                item={item}
                pending={action.isPending && action.variables?.hevyId === item.hevyId}
              >
                <ActionButton
                  label="Unskip"
                  busy="Restoring…"
                  disabled={action.isPending}
                  pending={action.isPending && action.variables?.hevyId === item.hevyId}
                  onClick={() => runAction(item, "unskip")}
                />
              </QueueRow>
            ))}
          </QueueSection>
        ) : null}
      </CardContent>
    </Card>
  );
}

function QueueSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="grid gap-3">
      <h3 className="text-sm font-semibold">{title}</h3>
      <div className="grid gap-2">{children}</div>
    </section>
  );
}

function QueueRow({
  item,
  pending,
  children,
}: {
  item: HevyQueueItem;
  pending: boolean;
  children?: React.ReactNode;
}) {
  return (
    <article className="grid gap-3 rounded-xl border bg-background/60 p-3 sm:grid-cols-[1fr_auto] sm:items-center">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <h4 className="truncate font-medium">{item.title}</h4>
          <Badge variant={item.error ? "destructive" : "secondary"}>
            {item.status.replaceAll("_", " ")}
          </Badge>
        </div>
        <p className="text-xs text-muted-foreground">{item.startDisplay}</p>
        {item.error ? (
          <p className="mt-1 text-sm text-destructive">{item.error}</p>
        ) : null}
      </div>
      {children ? (
        <div className="flex flex-wrap gap-2" aria-busy={pending}>
          {children}
        </div>
      ) : null}
    </article>
  );
}

function ActionButton({
  label,
  busy,
  disabled,
  pending,
  onClick,
}: {
  label: string;
  busy: string;
  disabled: boolean;
  pending: boolean;
  onClick: () => void;
}) {
  return (
    <Button variant="outline" size="sm" disabled={disabled} onClick={onClick}>
      {pending ? <Spinner data-icon="inline-start" /> : null}
      {pending ? busy : label}
    </Button>
  );
}
