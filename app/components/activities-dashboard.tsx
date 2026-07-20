import { useState } from "react";
import { ActivityIcon, TriangleAlertIcon } from "lucide-react";

import { ActivityCard } from "@/components/activity-card";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationLink,
  PaginationNext,
  PaginationPrevious,
} from "@/components/ui/pagination";
import {
  NativeSelect,
  NativeSelectOption,
} from "@/components/ui/native-select";
import { Card, CardContent } from "@/components/ui/card";
import { Field, FieldLabel } from "@/components/ui/field";
import { Spinner } from "@/components/ui/spinner";
import type {
  ActivityActionInput,
  ActivityActionState,
} from "@/hooks/use-activity-actions";
import type {
  ActivitiesPage,
  ActivityActionResult,
  ActivityQuery,
  AppState,
  PageSize,
  PublishStatus,
  SortOrder,
} from "@/lib/api";

const statusOptions: Array<{ value: PublishStatus; label: string }> = [
  { value: "pending", label: "Pending" },
  { value: "held", label: "Held" },
  { value: "published", label: "Published" },
  { value: "missing", label: "Missing" },
  { value: "excluded", label: "Excluded" },
];

const pageSizes: PageSize[] = [10, 20, 50, 100];

type ActivitiesDashboardProps = {
  appState: AppState;
  data: ActivitiesPage;
  query: ActivityQuery;
  actionState: ActivityActionState;
  onQueryChange: (next: Partial<ActivityQuery>) => void;
  onAction: (action: ActivityActionInput) => Promise<ActivityActionResult>;
};

export function ActivitiesDashboard({
  appState,
  data,
  query,
  actionState,
  onQueryChange,
  onAction,
}: ActivitiesDashboardProps) {
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<number>>(() => new Set());
  const groups = groupByMonth(data.items);
  const currentPage = data.pagination.page;
  const publishableIds = data.items
    .filter((activity) => isPublishable(activity.publishStatus))
    .map((activity) => activity.garminActivityId);
  const selectedOnPage = publishableIds.filter((activityId) =>
    selectedIds.has(activityId),
  );
  const publishingMany =
    actionState.isPending && actionState.action?.type === "publish-many";
  const allStatusesCount = Object.values(data.counts).reduce(
    (total, count) => total + count,
    0,
  );

  return (
    <div className="grid gap-6">
      {appState.connections.broken.length > 0 ? (
        <Alert variant="destructive">
          <TriangleAlertIcon />
          <AlertTitle>Sync needs attention</AlertTitle>
          <AlertDescription>
            Reconnect {appState.connections.broken.join(" and ")} in Settings.
            Activities remain available, but publishing is paused.
          </AlertDescription>
        </Alert>
      ) : null}

      <Card size="sm" role="group" aria-label="Activity view controls">
        <CardContent className="grid gap-3 sm:grid-cols-2 lg:grid-cols-[1fr_1fr_auto]">
          <NativeSelect
            className="w-full [&_select]:h-11"
            aria-label="Filter activities"
            value={query.status ?? "all"}
            onChange={(event) =>
              onQueryChange({
                status:
                  event.target.value === "all"
                    ? null
                    : (event.target.value as PublishStatus),
                page: 1,
              })
            }
          >
            <NativeSelectOption value="all">
              All statuses ({allStatusesCount})
            </NativeSelectOption>
            {statusOptions.map((option) => (
              <NativeSelectOption key={option.value} value={option.value}>
                {option.label} ({data.counts[option.value]})
              </NativeSelectOption>
            ))}
          </NativeSelect>

          <NativeSelect
            className="w-full [&_select]:h-11"
            aria-label="Sort activities"
            value={query.sort}
            onChange={(event) =>
              onQueryChange({
                sort: event.target.value as SortOrder,
                page: 1,
              })
            }
          >
            <NativeSelectOption value="newest">Newest first</NativeSelectOption>
            <NativeSelectOption value="oldest">Oldest first</NativeSelectOption>
          </NativeSelect>

          <p className="self-center px-1 text-sm text-muted-foreground lg:text-right">
            {data.pagination.totalCount === 0
              ? "No activities"
              : `${data.pagination.firstItem}–${data.pagination.lastItem} of ${data.pagination.totalCount}`}
          </p>
        </CardContent>
      </Card>

      {publishableIds.length > 0 ? (
        selectionMode ? (
          <Card size="sm" role="group" aria-label="Publish multiple activities">
            <CardContent className="flex flex-wrap items-center gap-3">
              <Field orientation="horizontal" className="w-auto">
                <Checkbox
                  id="select-all-activities"
                  checked={
                    selectedOnPage.length === publishableIds.length
                      ? true
                      : selectedOnPage.length > 0
                        ? "indeterminate"
                        : false
                  }
                  disabled={actionState.isPending}
                  onCheckedChange={(checked) =>
                    setSelectedIds(
                      checked === true ? new Set(publishableIds) : new Set(),
                    )
                  }
                />
                <FieldLabel htmlFor="select-all-activities">
                  Select all on page
                </FieldLabel>
              </Field>
              <span className="text-sm text-muted-foreground" role="status">
                {selectedOnPage.length} selected
              </span>
              <div className="ml-auto flex gap-2">
                <Button
                  variant="outline"
                  className="h-10"
                  disabled={actionState.isPending}
                  onClick={() => {
                    setSelectedIds(new Set());
                    setSelectionMode(false);
                  }}
                >
                  Cancel
                </Button>
                <Button
                  className="h-10"
                  disabled={
                    actionState.isPending ||
                    selectedOnPage.length === 0 ||
                    appState.connections.broken.length > 0
                  }
                  aria-busy={publishingMany}
                  onClick={() => {
                    void onAction({
                      type: "publish-many",
                      activityIds: selectedOnPage,
                    })
                      .then(() => {
                        setSelectedIds(new Set());
                        setSelectionMode(false);
                      })
                      .catch(() => undefined);
                  }}
                >
                  {publishingMany ? (
                    <>
                      <Spinner data-icon="inline-start" aria-hidden="true" />
                      <span className="text-xs">Publishing…</span>
                    </>
                  ) : (
                    "Publish selected"
                  )}
                </Button>
              </div>
            </CardContent>
          </Card>
        ) : (
          <Button
            variant="outline"
            className="h-10 w-fit"
            disabled={actionState.isPending}
            onClick={() => setSelectionMode(true)}
          >
            Select multiple
          </Button>
        )
      ) : null}

      {groups.length > 0 ? (
        <div className="grid gap-7" aria-label="Activities">
          {groups.map((group) => (
            <section key={group.month} aria-labelledby={`month-${group.key}`}>
              <h2
                id={`month-${group.key}`}
                className="mb-3 border-b pb-2 text-xs font-semibold tracking-[0.12em] text-muted-foreground uppercase"
              >
                {group.month}
              </h2>
              <div className="grid gap-3 lg:grid-cols-2">
                {group.activities.map((activity) => (
                  <ActivityCard
                    key={activity.garminActivityId}
                    activity={activity}
                    connectionsBroken={appState.connections.broken.length > 0}
                    selectionMode={selectionMode}
                    selected={selectedIds.has(activity.garminActivityId)}
                    actionState={actionState}
                    onSelectedChange={(selected) =>
                      setSelectedIds((current) => {
                        const next = new Set(current);
                        if (selected) {
                          next.add(activity.garminActivityId);
                        } else {
                          next.delete(activity.garminActivityId);
                        }
                        return next;
                      })
                    }
                    onAction={onAction}
                  />
                ))}
              </div>
            </section>
          ))}
        </div>
      ) : (
        <Empty className="min-h-64 border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ActivityIcon />
            </EmptyMedia>
            <EmptyTitle>No activities found</EmptyTitle>
            <EmptyDescription>
              {query.status
                ? `There are no ${query.status} activities in the current sync history.`
                : "Activities will appear here after the first Garmin sync."}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}

      <div className="grid gap-4 border-t pt-5 sm:grid-cols-[1fr_auto_1fr] sm:items-center">
        <div className="flex items-center gap-2">
          <span className="text-sm text-muted-foreground">Per page</span>
          <NativeSelect
            className="w-20 [&_select]:h-11"
            aria-label="Activities per page"
            value={String(query.pageSize)}
            onChange={(event) =>
              onQueryChange({
                pageSize: Number(event.target.value) as PageSize,
                page: 1,
              })
            }
          >
            {pageSizes.map((pageSize) => (
              <NativeSelectOption key={pageSize} value={String(pageSize)}>
                {pageSize}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </div>

        <Pagination className="sm:col-start-2">
          <PaginationContent>
            {currentPage > 1 ? (
              <PaginationItem>
                <PaginationPrevious
                  href={pageHref(query, currentPage - 1)}
                  className="h-11"
                  onClick={(event) => {
                    event.preventDefault();
                    onQueryChange({ page: currentPage - 1 });
                  }}
                />
              </PaginationItem>
            ) : null}
            <PaginationItem>
              <PaginationLink
                href={pageHref(query, currentPage)}
                isActive
                className="h-11 min-w-11"
                aria-label={`Page ${currentPage} of ${data.pagination.pageCount}`}
              >
                {currentPage}
              </PaginationLink>
            </PaginationItem>
            {currentPage < data.pagination.pageCount ? (
              <PaginationItem>
                <PaginationNext
                  href={pageHref(query, currentPage + 1)}
                  className="h-11"
                  onClick={(event) => {
                    event.preventDefault();
                    onQueryChange({ page: currentPage + 1 });
                  }}
                />
              </PaginationItem>
            ) : null}
          </PaginationContent>
        </Pagination>

        <p className="text-sm text-muted-foreground sm:col-start-3 sm:text-right">
          Page {currentPage} of {data.pagination.pageCount}
        </p>
      </div>
    </div>
  );
}

function isPublishable(status: PublishStatus) {
  return status === "pending" || status === "held" || status === "missing";
}

function groupByMonth(items: ActivitiesPage["items"]) {
  return items.reduce<
    Array<{
      month: string;
      key: string;
      activities: ActivitiesPage["items"];
    }>
  >((groups, activity) => {
    const previous = groups.at(-1);
    if (previous?.month === activity.startMonthYearDisplay) {
      return [
        ...groups.slice(0, -1),
        { ...previous, activities: [...previous.activities, activity] },
      ];
    }
    return [
      ...groups,
      {
        month: activity.startMonthYearDisplay,
        key: activity.startMonthYearDisplay.toLowerCase().replaceAll(" ", "-"),
        activities: [activity],
      },
    ];
  }, []);
}

function pageHref(query: ActivityQuery, page: number) {
  const search = new URLSearchParams({
    sort: query.sort,
    page: String(page),
    pageSize: String(query.pageSize),
  });
  if (query.status !== null) {
    search.set("status", query.status);
  }
  return `/?${search}`;
}
