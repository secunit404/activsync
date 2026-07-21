import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ActivityIcon } from "lucide-react";
import { useEffect } from "react";
import { Navigate, Outlet, useNavigate, useOutletContext, useSearchParams } from "react-router";

import { ActivitiesPagination } from "@/components/activities-pagination";
import { ActivitiesSkeleton } from "@/components/activities-skeleton";
import { ActivitiesTable } from "@/components/activities-table";
import { ActivityCard } from "@/components/activity-card";
import { ActivityFilterPills } from "@/components/activity-filter-pills";
import { AttentionBanner } from "@/components/attention-banner";
import { BulkActionBar } from "@/components/bulk-action-bar";
import { CatchUpReport } from "@/components/catch-up-report";
import { ConnectionError } from "@/components/connection-error";
import { StatTile, statTileToneClass, type StatTileTone } from "@/components/stat-tile";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { useActivityActions } from "@/hooks/use-activity-actions";
import { useLiveRefresh } from "@/hooks/use-live-refresh";
import { useSelection } from "@/hooks/use-selection";
import {
  dismissCatchUpReport,
  getActivities,
  getAppState,
  type ActivitiesPage,
  type ActivityQuery,
  type AppState,
  type PageSize,
  type PublishStatus,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import type { Route } from "./+types/activities";

/** Threaded down from `AppLayout` via `Outlet` context — see its docstring. */
type TabBarVisibilityContext = (hidden: boolean) => void;

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
  const setTabBarHidden = useOutletContext<TabBarVisibilityContext>();

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
    return (
      <div className="p-6 md:p-8">
        <ConnectionError
          onRetry={() => {
            appState.refetch();
            activities.refetch();
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
      <ActivitiesView
        data={activities.data}
        appState={appState.data}
        onTabBarHiddenChange={setTabBarHidden}
      />
      <Outlet />
    </>
  );
}

export function ActivitiesView({
  data,
  appState,
  onTabBarHiddenChange,
}: {
  data: ActivitiesPage;
  /**
   * Optional so `ActivitiesView` stays directly renderable in tests without
   * an `AppLayout`/`Outlet` ancestor (see activities.test.tsx), and without
   * every test needing a full `AppState` fixture when the attention banner
   * and catch-up report are irrelevant to what's being tested — defaults to
   * "nothing broken, nothing to catch up on".
   */
  appState?: Pick<AppState, "connections" | "catchUpReport">;
  onTabBarHiddenChange?: TabBarVisibilityContext;
}) {
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const selection = useSelection();
  const activityActions = useActivityActions();
  const dismissCatchUp = useMutation({
    mutationFn: dismissCatchUpReport,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.appState }),
  });
  const broken = appState?.connections.broken ?? [];
  const catchUpReport = appState?.catchUpReport ?? null;

  // Selection must not survive a filter or page change — a stale id from a
  // page the user has since left would publish/exclude something they can
  // no longer see. Keyed on the search-param string (not `data`), so a
  // same-query background refetch (live-refresh SSE, a poll) does not clear
  // an in-progress selection out from under the user.
  useEffect(() => {
    selection.clear();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams.toString()]);

  // Tell AppLayout to hide the mobile tab bar for as long as the bar is
  // occupying its slot, and restore it on unmount (navigating to another
  // top-level route) so it never gets stuck hidden.
  useEffect(() => {
    onTabBarHiddenChange?.(selection.count > 0);
    return () => onTabBarHiddenChange?.(false);
  }, [selection.count, onTabBarHiddenChange]);

  const toggleAll = () => {
    selection.toggleAll(data.items.map((activity) => activity.garminActivityId));
  };

  const selectedActivities = data.items.filter((activity) =>
    selection.selected.has(activity.garminActivityId),
  );

  const handlePublish = () => {
    activityActions.mutate(
      { type: "publish-many", activityIds: Array.from(selection.selected) },
      { onSuccess: () => selection.clear() },
    );
  };

  const handleExclude = () => {
    activityActions.mutate(
      { type: "exclude-many", activityIds: Array.from(selection.selected) },
      { onSuccess: () => selection.clear() },
    );
  };

  const handleReconnect = () => navigate("/settings#connections");

  return (
    <div
      className={cn(
        "grid gap-6 p-6 md:gap-7 md:p-8",
        // The mobile bulk bar is fixed-bottom, same slot as the tab bar
        // (which AppLayout already reserves 74px for); give the page a
        // little extra clearance only while it's actually showing, so the
        // last table row/card isn't tucked underneath it.
        selection.count > 0 && "pb-32 md:pb-8",
      )}
    >
      <header className="grid gap-1">
        <h1 className="text-2xl font-extrabold tracking-[-0.02em] sm:text-[26px]">
          Activities
        </h1>
        <p className="text-sm text-muted-foreground">
          Review your Garmin sync history before it reaches Strava.
        </p>
      </header>

      {
        // One banner per broken service — `broken` can hold both "garmin"
        // and "strava" at once, and each needs its own reconnect copy (see
        // AttentionBanner), so this maps rather than picking just the first.
        broken.map((service) => (
          <AttentionBanner key={service} service={service} onReconnect={handleReconnect} />
        ))
      }

      <CatchUpReport report={catchUpReport} onDismiss={() => dismissCatchUp.mutate()} />

      <StatStrip data={data} />

      <ActivityFilterPills counts={data.counts} />

      <BulkActionBar
        count={selection.count}
        names={selectedActivities.map((activity) => activity.title)}
        onClear={selection.clear}
        onExclude={handleExclude}
        onPublish={handlePublish}
        busy={activityActions.isPending}
      />

      {data.items.length > 0 ? (
        <>
          <ActivitiesTable
            activities={data.items}
            selected={selection.selected}
            onToggle={selection.toggle}
            onToggleAll={toggleAll}
          />
          <div data-testid="activities-cards" className="grid gap-3 md:hidden">
            {data.items.map((activity) => (
              <ActivityCard
                key={activity.garminActivityId}
                activity={activity}
                selected={selection.selected.has(activity.garminActivityId)}
                onToggle={selection.toggle}
              />
            ))}
          </div>
        </>
      ) : (
        <Empty className="min-h-64">
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
