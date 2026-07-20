import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleAlertIcon } from "lucide-react";
import { Link, useSearchParams } from "react-router";

import { ActivitiesDashboard } from "@/components/activities-dashboard";
import {
  AppBrand,
  AppFooter,
  AppShell,
  AppShellHeader,
  AppShellMain,
} from "@/components/app-shell";
import { HevyQueue } from "@/components/hevy-queue";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import {
  useActivityActions,
  type ActivityActionInput,
  type ActivityActionState,
} from "@/hooks/use-activity-actions";
import {
  dismissCatchUpReport,
  getActivities,
  getAppState,
  getHevyQueue,
  type ActivitiesPage,
  type ActivityActionResult,
  type ActivityQuery,
  type AppState,
  type HevyQueueState,
  type PageSize,
  type PublishStatus,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { Route } from "./+types/home";

const publishStatuses = new Set<PublishStatus>([
  "pending",
  "held",
  "published",
  "missing",
  "excluded",
]);
const pageSizes = new Set<PageSize>([10, 20, 50, 100]);
const emptyHevyQueue: HevyQueueState = {
  enabled: false,
  inFlight: [],
  problems: [],
  skipped: [],
  counts: { inFlight: 0, problems: 0, skipped: 0 },
};

export function meta(): Route.MetaDescriptors {
  return [
    { title: "Activities · ActivSync" },
    {
      name: "description",
      content: "Review and publish Garmin activities to Strava.",
    },
  ];
}

export default function Home() {
  const queryClient = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();
  const query = activityQueryFrom(searchParams);
  const activityActions = useActivityActions();
  const appState = useQuery({
    queryKey: queryKeys.appState,
    queryFn: ({ signal }) => getAppState(signal),
  });
  const activities = useQuery({
    queryKey: queryKeys.activities(query),
    queryFn: ({ signal }) => getActivities(query, signal),
  });
  const hevyQueue = useQuery({
    queryKey: queryKeys.hevyQueue,
    queryFn: ({ signal }) => getHevyQueue(signal),
  });
  const dismissReport = useMutation({
    mutationFn: dismissCatchUpReport,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.appState });
    },
  });

  if (appState.isPending || activities.isPending || hevyQueue.isPending) {
    return <HomeSkeleton />;
  }

  if (appState.isError || activities.isError || hevyQueue.isError) {
    const error = appState.error ?? activities.error ?? hevyQueue.error;
    return (
      <AppShell>
        <AppShellHeader>
          <AppBrand />
        </AppShellHeader>
        <AppShellMain>
          <Alert variant="destructive" className="max-w-xl">
            <CircleAlertIcon />
            <AlertTitle>Couldn’t reach ActivSync</AlertTitle>
            <AlertDescription>
              {error?.message ?? "An unexpected request error occurred."}
            </AlertDescription>
          </Alert>
          <Button
            className="mt-4 h-11"
            onClick={() => {
              appState.refetch();
              activities.refetch();
              hevyQueue.refetch();
            }}
          >
            Try again
          </Button>
        </AppShellMain>
      </AppShell>
    );
  }

  return (
    <HomeView
      state={appState.data}
      activities={activities.data}
      hevyQueue={hevyQueue.data}
      query={query}
      actionState={{
        isPending: activityActions.isPending,
        action: activityActions.variables,
      }}
      onActivityAction={activityActions.mutateAsync}
      dismissingCatchUp={dismissReport.isPending}
      onDismissCatchUp={() => dismissReport.mutateAsync()}
      onQueryChange={(patch) => {
        const next = { ...query, ...patch };
        setSearchParams(activitySearchParams(next));
      }}
    />
  );
}

type HomeViewProps = {
  state: AppState;
  activities: ActivitiesPage;
  hevyQueue?: HevyQueueState;
  query: ActivityQuery;
  actionState: ActivityActionState;
  onActivityAction: (
    action: ActivityActionInput,
  ) => Promise<ActivityActionResult>;
  dismissingCatchUp?: boolean;
  onDismissCatchUp?: () => Promise<void>;
  onQueryChange: (next: Partial<ActivityQuery>) => void;
};

export function HomeView({
  state,
  activities,
  hevyQueue = emptyHevyQueue,
  query,
  actionState,
  onActivityAction,
  dismissingCatchUp = false,
  onDismissCatchUp = async () => undefined,
  onQueryChange,
}: HomeViewProps) {
  return (
    <AppShell>
      <AppShellHeader>
        <div className="flex items-center gap-3">
          <AppBrand />
          {state.development ? <Badge variant="outline">Mock data</Badge> : null}
        </div>
        <Button asChild variant="ghost" className="h-11 px-3">
          <Link to="/settings">Settings</Link>
        </Button>
      </AppShellHeader>

      <AppShellMain className="grid gap-8">
        <section className="grid gap-4 sm:grid-cols-[1fr_auto] sm:items-end">
          <div className="grid gap-2">
            <p className="text-xs font-bold tracking-[0.14em] text-[var(--sync)] uppercase">
              Garmin → Strava
            </p>
            <h1 className="text-4xl leading-none font-bold tracking-[-0.05em] sm:text-5xl">
              Activities
            </h1>
            <p className="text-base text-muted-foreground">
              Review your Garmin sync history before publishing to Strava.
            </p>
          </div>
          <div className="flex flex-wrap gap-2" aria-label="Connection status">
            <Badge variant="secondary">
              Garmin {state.connections.garmin.connected ? "ready" : "paused"}
            </Badge>
            <Badge variant="secondary">
              Strava {state.connections.strava.connected ? "ready" : "paused"}
            </Badge>
            {state.hevy.enabled ? (
              <Badge variant="secondary">
                Hevy {state.hevy.connected ? "ready" : "paused"}
              </Badge>
            ) : null}
          </div>
        </section>

        {!state.setup.complete ? (
          <Card className="max-w-2xl border-[var(--sync)]/35">
            <CardHeader>
              <CardTitle>Finish setup</CardTitle>
              <CardDescription>
                Continue the existing {state.setup.step} step before activities
                can sync.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Button asChild className="h-11 px-4">
                <Link to="/setup">Continue setup</Link>
              </Button>
            </CardContent>
          </Card>
        ) : (
          <>
            {state.catchUpReport ? (
              <Alert>
                <AlertTitle>Reconnect catch-up complete</AlertTitle>
                <AlertDescription className="grid gap-3 sm:grid-cols-[1fr_auto] sm:items-center">
                  <span>
                    Found {state.catchUpReport.new} activities across the last {" "}
                    {state.catchUpReport.days} days; {state.catchUpReport.linked} {" "}
                    already existed on Strava and {state.catchUpReport.held} are held
                    for review.
                  </span>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={dismissingCatchUp}
                    onClick={() => void onDismissCatchUp()}
                  >
                    {dismissingCatchUp ? <Spinner data-icon="inline-start" /> : null}
                    {dismissingCatchUp ? "Dismissing…" : "Dismiss"}
                  </Button>
                </AlertDescription>
              </Alert>
            ) : null}
            <HevyQueue state={hevyQueue} />
            <ActivitiesDashboard
              appState={state}
              data={activities}
              query={query}
              actionState={actionState}
              onQueryChange={onQueryChange}
              onAction={onActivityAction}
            />
          </>
        )}
      </AppShellMain>
      <AppFooter version={state.version} update={state.update} />
    </AppShell>
  );
}

function HomeSkeleton() {
  return (
    <AppShell>
      <AppShellHeader>
        <AppBrand />
      </AppShellHeader>
      <AppShellMain className="grid gap-6">
        <div className="grid gap-3">
          <Skeleton className="h-4 w-28" />
          <Skeleton className="h-12 w-56" />
          <Skeleton className="h-6 w-full max-w-xl" />
        </div>
        <Skeleton className="h-18 w-full" />
        <div className="grid gap-3 lg:grid-cols-2">
          {Array.from({ length: 6 }, (_, index) => (
            <Skeleton key={index} className="h-44 w-full" />
          ))}
        </div>
      </AppShellMain>
    </AppShell>
  );
}

function activityQueryFrom(searchParams: URLSearchParams): ActivityQuery {
  const rawStatus = searchParams.get("status");
  const rawSort = searchParams.get("sort");
  const rawPage = Number(searchParams.get("page"));
  const rawPageSize = Number(searchParams.get("pageSize"));

  return {
    sort: rawSort === "oldest" ? "oldest" : "newest",
    status:
      rawStatus !== null && publishStatuses.has(rawStatus as PublishStatus)
        ? (rawStatus as PublishStatus)
        : null,
    page: Number.isInteger(rawPage) && rawPage > 0 ? rawPage : 1,
    pageSize: pageSizes.has(rawPageSize as PageSize)
      ? (rawPageSize as PageSize)
      : 20,
  };
}

function activitySearchParams(query: ActivityQuery) {
  const search = new URLSearchParams();
  if (query.sort !== "newest") {
    search.set("sort", query.sort);
  }
  if (query.status !== null) {
    search.set("status", query.status);
  }
  if (query.page !== 1) {
    search.set("page", String(query.page));
  }
  if (query.pageSize !== 20) {
    search.set("pageSize", String(query.pageSize));
  }
  return search;
}
