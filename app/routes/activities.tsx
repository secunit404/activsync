import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ActivityIcon } from "lucide-react";
import { useEffect } from "react";
import { Navigate, Outlet, useNavigate, useOutletContext, useSearchParams } from "react-router";

import { ActivitiesPagination } from "@/components/activities-pagination";
import { ActivitiesSkeleton } from "@/components/activities-skeleton";
import { ActivitiesTable } from "@/components/activities-table";
import { ActivityCard } from "@/components/activity-card";
import { ActivityFilterPills } from "@/components/activity-filter-pills";
import { ActivitySortSelect } from "@/components/activity-sort-select";
import { AttentionBanner } from "@/components/attention-banner";
import { BulkActionBar } from "@/components/bulk-action-bar";
import { CatchUpReport } from "@/components/catch-up-report";
import { ConnectionError } from "@/components/connection-error";
import { StatTile, statTileToneClass, type StatTileTone } from "@/components/stat-tile";
import { isExcludableStatus, isPublishableStatus } from "@/lib/activity-metrics";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Spinner } from "@/components/ui/spinner";
import { useActivityActions } from "@/hooks/use-activity-actions";
import { useLiveRefresh } from "@/hooks/use-live-refresh";
import { useSelection } from "@/hooks/use-selection";
import {
  ApiError,
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
    // Keep showing the previous page/filter's rows while the new query key's
    // data loads, instead of unmounting the whole page to a skeleton on
    // every click. With this, `isPending` (no data at all yet) only stays
    // true for the genuine first load — a filter/page change instead flips
    // `isFetching` while `data` keeps the last-known rows, which the view
    // reflects with a subtle indicator rather than the full skeleton.
    placeholderData: keepPreviousData,
  });

  if (appState.isPending || activities.isPending) {
    return <ActivitiesSkeleton />;
  }

  if (appState.isError || activities.isError) {
    // Either query's error can be an ApiError (the backend responded with a
    // non-2xx status and a `detail`) or a plain fetch failure (the request
    // never reached the backend at all) — ConnectionError needs to tell
    // those apart rather than always claiming the server didn't respond.
    const error = appState.error ?? activities.error;
    const apiError = error instanceof ApiError ? error : undefined;
    return (
      <div className="p-6 md:p-8">
        <ConnectionError
          status={apiError?.status}
          detail={apiError?.message}
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
        // True only for a background refetch of a *new* query key (filter/
        // page change) that's still resolving behind the kept-previous data
        // — not for the initial load (already handled above) and not for a
        // same-key background refetch (SSE/poll), which should stay silent.
        isRefetching={activities.isFetching && activities.isPlaceholderData}
      />
      {
        // The activity detail route (Task 11) needs the current page's
        // activities to resolve `:id` to an `Activity` without a second
        // fetch — the list is already loaded and stays mounted behind the
        // overlay. Threaded via `Outlet`'s `context` prop, same pattern
        // AppLayout already uses one level up for tab-bar visibility.
      }
      <Outlet context={activities.data.items} />
    </>
  );
}

export function ActivitiesView({
  data,
  appState,
  onTabBarHiddenChange,
  isRefetching = false,
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
  /** True while a filter/page change is loading behind the kept-previous data. */
  isRefetching?: boolean;
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

  const toggleAll = () => {
    selection.toggleAll(data.items.map((activity) => activity.garminActivityId));
  };

  // Everything downstream — the bulk action ids, the displayed count, the
  // tab-bar/padding "is the bar showing" state — derives from this filtered
  // list, not the raw `selection.selected` id set. `selection.selected` can
  // outlive its row: an SSE-driven background refetch can reclassify an
  // activity out of the current filter while its id is still selected
  // (selection only clears on a filter/page change, see the effect above),
  // and a bulk action must never fire on an id for a row the user can no
  // longer see.
  const selectedActivities = data.items.filter((activity) =>
    selection.selected.has(activity.garminActivityId),
  );
  const selectedCount = selectedActivities.length;

  // Bulk actions fire on the subset the server would accept, not the whole
  // selection — mirrors `api_routes.py`'s per-activity status guards, so a
  // mixed selection can no longer produce a 409 the user reads as failure.
  const excludableIds = selectedActivities
    .filter((activity) => isExcludableStatus(activity.publishStatus))
    .map((activity) => activity.garminActivityId);
  const publishableIds = selectedActivities
    .filter((activity) => isPublishableStatus(activity.publishStatus))
    .map((activity) => activity.garminActivityId);

  // Tell AppLayout to hide the mobile tab bar for as long as the bar is
  // occupying its slot, and restore it on unmount (navigating to another
  // top-level route) so it never gets stuck hidden.
  useEffect(() => {
    onTabBarHiddenChange?.(selectedCount > 0);
    return () => onTabBarHiddenChange?.(false);
  }, [selectedCount, onTabBarHiddenChange]);

  const handlePublish = () => {
    activityActions.mutate(
      { type: "publish-many", activityIds: publishableIds },
      { onSuccess: () => selection.clear() },
    );
  };

  const handleExclude = () => {
    activityActions.mutate(
      { type: "exclude-many", activityIds: excludableIds },
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
        selectedCount > 0 && "pb-32 md:pb-8",
      )}
    >
      <header className="grid gap-1">
        <h1 className="flex items-center gap-2 text-2xl font-extrabold tracking-[-0.02em] sm:text-[26px]">
          Activities
          {isRefetching ? (
            <Spinner
              aria-label="Loading activities"
              className="size-4 text-muted-foreground"
            />
          ) : null}
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

      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <ActivityFilterPills counts={data.counts} />
        </div>
        <ActivitySortSelect />
      </div>

      <BulkActionBar
        count={selectedCount}
        excludableCount={excludableIds.length}
        publishableCount={publishableIds.length}
        onClear={selection.clear}
        onExclude={handleExclude}
        onPublish={handlePublish}
        busy={activityActions.isPending}
        // Only a broken Strava connection pauses publishing (see
        // AttentionBanner's copy: a broken Garmin connection still lets
        // publishing continue for activities already synced). Exclude is a
        // purely local write, so it stays enabled either way.
        publishDisabled={broken.includes("strava")}
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
