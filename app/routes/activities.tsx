import { useQuery } from "@tanstack/react-query";
import { ActivityIcon, CircleAlertIcon } from "lucide-react";
import { useState } from "react";
import { Navigate, Outlet, useSearchParams } from "react-router";

import { ActivitiesPagination } from "@/components/activities-pagination";
import { ActivitiesTable } from "@/components/activities-table";
import { ActivityCard } from "@/components/activity-card";
import { ActivityFilterPills } from "@/components/activity-filter-pills";
import { StatTile, statTileToneClass, type StatTileTone } from "@/components/stat-tile";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { useLiveRefresh } from "@/hooks/use-live-refresh";
import {
  getActivities,
  getAppState,
  type ActivitiesPage,
  type ActivityQuery,
  type PageSize,
  type PublishStatus,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import type { Route } from "./+types/activities";

const publishStatuses = new Set<PublishStatus>([
  "pending",
  "held",
  "published",
  "missing",
  "excluded",
]);
const pageSizes = new Set<PageSize>([10, 20, 50, 100]);

export function meta(): Route.MetaDescriptors {
  return [
    { title: "Activities · ActivSync" },
    {
      name: "description",
      content: "Review and publish Garmin activities to Strava.",
    },
  ];
}

export default function Activities() {
  useLiveRefresh();
  const [searchParams] = useSearchParams();
  const query = activityQueryFrom(searchParams);

  // appState is fetched here (not just in app-layout.tsx) so this route can
  // apply the same "redirect to /setup when incomplete" gate settings.tsx
  // already uses — the shared queryKeys.appState cache means this is a
  // no-op refetch once app-layout's query has resolved.
  const appState = useQuery({
    queryKey: queryKeys.appState,
    queryFn: ({ signal }) => getAppState(signal),
  });
  const activities = useQuery({
    queryKey: queryKeys.activities(query),
    queryFn: ({ signal }) => getActivities(query, signal),
  });

  if (appState.isPending || activities.isPending) {
    return <ActivitiesSkeleton />;
  }

  if (appState.isError || activities.isError) {
    const error = appState.error ?? activities.error;
    return (
      <div className="grid gap-4 p-6 md:p-8">
        <Alert variant="destructive" className="max-w-xl">
          <CircleAlertIcon />
          <AlertTitle>Couldn’t reach ActivSync</AlertTitle>
          <AlertDescription>
            {error?.message ?? "An unexpected request error occurred."}
          </AlertDescription>
        </Alert>
        <Button
          className="h-11 w-fit"
          onClick={() => {
            appState.refetch();
            activities.refetch();
          }}
        >
          Try again
        </Button>
      </div>
    );
  }

  if (!appState.data.setup.complete) {
    return <Navigate to="/setup" replace />;
  }

  return (
    <>
      <ActivitiesView data={activities.data} />
      <Outlet />
    </>
  );
}

export function ActivitiesView({ data }: { data: ActivitiesPage }) {
  // Placeholder selection state so ActivitiesTable/ActivityCard's
  // selected/onToggle/onToggleAll contract has something to bind to. Task 9
  // owns the real `useSelection` hook (and the bulk-action bar it drives) —
  // this local, immutable-Set implementation is a stand-in with the exact
  // same shape, kept here only so the checkboxes are interactive.
  const [selected, setSelected] = useState<ReadonlySet<number>>(() => new Set());

  const toggle = (id: number) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const toggleAll = () => {
    setSelected((current) =>
      current.size === data.items.length
        ? new Set()
        : new Set(data.items.map((activity) => activity.garminActivityId)),
    );
  };

  return (
    <div className="grid gap-6 p-6 md:gap-7 md:p-8">
      <header className="grid gap-1">
        <h1 className="text-2xl font-extrabold tracking-[-0.02em] sm:text-[26px]">
          Activities
        </h1>
        <p className="text-sm text-muted-foreground">
          Review your Garmin sync history before it reaches Strava.
        </p>
      </header>

      <StatStrip data={data} />

      <ActivityFilterPills counts={data.counts} />

      {data.items.length > 0 ? (
        <>
          <ActivitiesTable
            activities={data.items}
            selected={selected}
            onToggle={toggle}
            onToggleAll={toggleAll}
          />
          <div data-testid="activities-cards" className="grid gap-3 md:hidden">
            {data.items.map((activity) => (
              <ActivityCard
                key={activity.garminActivityId}
                activity={activity}
                selected={selected.has(activity.garminActivityId)}
                onToggle={toggle}
              />
            ))}
          </div>
        </>
      ) : (
        <Empty className="min-h-64 border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ActivityIcon />
            </EmptyMedia>
            <EmptyTitle>No activities found</EmptyTitle>
            <EmptyDescription>
              {data.status
                ? `There are no ${data.status} activities in the current sync history.`
                : "Activities will appear here after the first Garmin sync."}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}

      <ActivitiesPagination page={data.pagination.page} pageCount={data.pagination.pageCount} />
    </div>
  );
}

type StatDefinition = { label: string; value: string; tone: StatTileTone };

function statDefinitions(data: ActivitiesPage): StatDefinition[] {
  return [
    { label: "PENDING", value: String(data.counts.pending), tone: "pending" },
    { label: "HELD", value: String(data.counts.held), tone: "held" },
    { label: "PUBLISHED", value: String(data.counts.published), tone: "published" },
    { label: "THIS WEEK", value: data.weekTotal.display, tone: "neutral" },
  ];
}

/**
 * Two parallel layouts switched by CSS breakpoint (never JS — see
 * ResponsiveOverlay's precedent): a tablet/desktop grid of individually
 * bordered StatTile cards (2-up at the 834px tablet width per handoff frame
 * `5c`, 4-up from `lg:` up per frame `1a`), and a mobile single card with a
 * divided flex row of compact cells, matching the mobile `1a` frame exactly
 * rather than reusing the desktop card shape at a smaller size.
 */
function StatStrip({ data }: { data: ActivitiesPage }) {
  const stats = statDefinitions(data);

  return (
    <section aria-label="Activity totals">
      <div
        data-testid="stat-tiles-grid"
        className="hidden grid-cols-2 gap-3 md:grid lg:grid-cols-4 lg:gap-3.5"
      >
        {stats.map((stat) => (
          <StatTile key={stat.label} label={stat.label} value={stat.value} tone={stat.tone} />
        ))}
      </div>

      <div
        data-testid="stat-tiles-strip"
        className="flex divide-x divide-border rounded-xl border border-border bg-card md:hidden"
      >
        {stats.map((stat) => (
          <div key={stat.label} className="flex flex-1 flex-col items-center gap-1 px-1 py-2.5">
            <span className="font-mono text-[9px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
              {stat.label}
            </span>
            <span
              className={cn(
                "font-mono leading-none font-extrabold",
                stat.tone === "neutral" ? "text-sm" : "text-lg",
                statTileToneClass[stat.tone],
              )}
            >
              {stat.value}
            </span>
          </div>
        ))}
      </div>
    </section>
  );
}

function ActivitiesSkeleton() {
  return (
    <div className="grid gap-6 p-6 md:gap-7 md:p-8">
      <div className="grid gap-2">
        <Skeleton className="h-8 w-40" />
        <Skeleton className="h-4 w-72" />
      </div>
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {Array.from({ length: 4 }, (_, index) => (
          <Skeleton key={index} className="h-24 w-full" />
        ))}
      </div>
      <Skeleton className="h-12 w-full" />
      <Skeleton className="h-64 w-full" />
    </div>
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
    pageSize: pageSizes.has(rawPageSize as PageSize) ? (rawPageSize as PageSize) : 20,
  };
}
