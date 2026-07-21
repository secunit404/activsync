import { useQuery } from "@tanstack/react-query";
import { Dumbbell } from "lucide-react";
import { Link, Navigate, Outlet } from "react-router";

import { ConnectionError } from "@/components/connection-error";
import { HevyBackfillCard } from "@/components/hevy-backfill-card";
import { HevyMappingSummary } from "@/components/hevy-mapping-summary";
import { HevyQueue } from "@/components/hevy-queue";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Spinner } from "@/components/ui/spinner";
import {
  ApiError,
  getAppState,
  getHevyQueue,
  getHevyTools,
  type AppState,
  type HevyQueueState,
  type HevyToolsState,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import type { Route } from "./+types/hevy";

export function meta(): Route.MetaDescriptors {
  return [
    { title: "Hevy · ActivSync" },
    {
      name: "description",
      content: "Hevy sync queue, exercise mapping, and historical backfill.",
    },
  ];
}

/**
 * The Hevy hub (frame `3a`): sync queue, exercise-mapping summary, and the
 * backfill entry point. `<Outlet />` mounts the mapping-editor (`4a`, Task
 * 15) and backfill (`4b`, Task 16) overlays over this page.
 *
 * `appState`, `hevyQueue`, and `hevyTools` are fetched independently
 * (mirroring `activities.tsx`'s pattern) rather than via a single combined
 * endpoint — each already exists as its own query key, and this route is
 * the first thing to read all three together.
 */
export default function Hevy() {
  const appState = useQuery({
    queryKey: queryKeys.appState,
    queryFn: ({ signal }) => getAppState(signal),
  });
  const queue = useQuery({
    queryKey: queryKeys.hevyQueue,
    queryFn: ({ signal }) => getHevyQueue(signal),
  });
  const tools = useQuery({
    queryKey: queryKeys.hevyTools,
    queryFn: ({ signal }) => getHevyTools(signal),
  });

  if (appState.isPending || queue.isPending || tools.isPending) {
    return (
      <div className="grid min-h-64 place-items-center p-16 text-muted-foreground">
        <Spinner className="size-6" aria-label="Loading Hevy" />
      </div>
    );
  }

  if (appState.isError || queue.isError || tools.isError) {
    const error = appState.error ?? queue.error ?? tools.error;
    const apiError = error instanceof ApiError ? error : undefined;
    return (
      <div className="p-6 md:p-8">
        <ConnectionError
          status={apiError?.status}
          detail={apiError?.message}
          onRetry={() => {
            appState.refetch();
            queue.refetch();
            tools.refetch();
          }}
        />
      </div>
    );
  }

  if (!appState.data.setup.complete) {
    return <Navigate to="/setup" replace />;
  }

  return (
    <>
      <HevyView appState={appState.data} queue={queue.data} tools={tools.data} />
      <Outlet />
    </>
  );
}

export function HevyView({
  appState,
  queue,
  tools,
}: {
  appState: Pick<AppState, "hevy">;
  queue: HevyQueueState;
  tools: HevyToolsState;
}) {
  const { connected, status } = appState.hevy;

  return (
    <div className="grid gap-6 p-6 md:gap-7 md:p-8">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div className="grid gap-1">
          <h1 className="text-2xl font-extrabold tracking-[-0.02em] sm:text-[26px]">Hevy</h1>
          <p className="text-sm text-muted-foreground">
            Workouts flowing from Hevy into Garmin, then on to Strava.
          </p>
        </div>
        <span className="flex items-center gap-2 rounded-[11px] border border-border bg-card px-3.5 py-2">
          <span
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              connected ? "bg-success" : "bg-muted-foreground/50",
            )}
            aria-hidden="true"
          />
          <span className="font-mono text-xs text-muted-foreground uppercase">{status}</span>
        </span>
      </header>

      {connected ? (
        <>
          <HevyQueue state={queue} />
          <HevyMappingSummary tools={tools} />
          <HevyBackfillCard />
        </>
      ) : (
        <Empty className="min-h-64">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Dumbbell />
            </EmptyMedia>
            <EmptyTitle>Connect Hevy to see your sync queue</EmptyTitle>
            <EmptyDescription>
              Once Hevy is connected in Settings, the sync queue, exercise
              mapping, and backfill tools will show up here.
            </EmptyDescription>
          </EmptyHeader>
          <EmptyContent>
            <Button asChild>
              <Link to="/settings#connections">Go to Settings</Link>
            </Button>
          </EmptyContent>
        </Empty>
      )}
    </div>
  );
}
