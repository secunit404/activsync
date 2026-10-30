import { ChevronRightIcon } from "lucide-react";
import { Link, useLocation, useNavigate } from "react-router";

import { StatusPill } from "@/components/status-pill";
import { TypePill } from "@/components/type-pill";
import { Checkbox } from "@/components/ui/checkbox";
import { selectAllState } from "@/hooks/use-selection";
import { activityEffort } from "@/lib/activity-metrics";
import { rowReason } from "@/lib/activity-status";
import type { Activity } from "@/lib/api";
import { cn } from "@/lib/utils";
import { MetaList } from "@/components/meta-list";

type ActivitiesTableProps = {
  activities: Activity[];
  selected: ReadonlySet<number>;
  publishingIds?: ReadonlySet<number>;
  onToggle: (id: number) => void;
  onToggleAll: () => void;
};

/**
 * Dense desktop/tablet table for the Activities screen (handoff frame
 * `1a`/`5c`). Real `<table>` markup — this is tabular data and screen
 * readers need `<th scope="col">`, not a div grid. Hidden below `md:`; the
 * sibling `ActivityCard` list covers the mobile viewport instead.
 */
export function ActivitiesTable({
  activities,
  selected,
  publishingIds = new Set(),
  onToggle,
  onToggleAll,
}: ActivitiesTableProps) {
  return (
    <div
      data-testid="activities-table"
      className="hidden overflow-hidden rounded-xl border border-border md:block"
    >
      <table className="w-full border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-border bg-card font-mono text-xs tracking-[0.08em] text-muted-foreground">
            <th scope="col" className="w-[38px] px-4 py-3 pointer-coarse:py-3.5">
              <Checkbox
                aria-label="Select all activities"
                checked={selectAllState(
                  activities.map((activity) => activity.garminActivityId),
                  selected,
                )}
                onCheckedChange={onToggleAll}
              />
            </th>
            <th scope="col" className="px-2 py-3 font-medium">
              ACTIVITY
            </th>
            <th scope="col" className="px-2 py-3 font-medium">
              TYPE
            </th>
            <th scope="col" className="px-2 py-3 font-medium">
              TIME
            </th>
            <th scope="col" className="px-2 py-3 font-medium">
              DISTANCE
            </th>
            <th scope="col" className="hidden px-2 py-3 font-medium lg:table-cell">
              EFFORT
            </th>
            <th scope="col" className="px-2 py-3 font-medium">
              STATUS
            </th>
            <th scope="col" className="w-[34px] px-2 py-3">
              <span className="sr-only">Open</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {activities.map((activity) => (
            <ActivityRow
              key={activity.garminActivityId}
              activity={activity}
              selected={selected.has(activity.garminActivityId)}
              publishing={publishingIds.has(activity.garminActivityId)}
              onToggle={onToggle}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ActivityRow({
  activity,
  selected,
  publishing,
  onToggle,
}: {
  activity: Activity;
  selected: boolean;
  publishing: boolean;
  onToggle: (id: number) => void;
}) {
  const navigate = useNavigate();
  const { search } = useLocation();
  const href = { pathname: `/${activity.garminActivityId}`, search };
  const effort = activityEffort(activity);
  const reason = rowReason(activity);

  return (
    <tr
      className={cn(
        "cursor-pointer border-b border-border/60 last:border-b-0 hover:bg-foreground/[0.04]",
        selected && "bg-primary/5",
      )}
      onClick={() => navigate(href)}
    >
      <td className="px-4 py-3" onClick={(event) => event.stopPropagation()}>
        <Checkbox
          data-activity-checkbox=""
          aria-label={`Select ${activity.title}`}
          checked={selected}
          onCheckedChange={() => onToggle(activity.garminActivityId)}
        />
      </td>
      <td className="min-w-0 px-2 py-3">
        <div className="flex min-w-0 flex-col gap-0.5">
          <Link
            to={href}
            data-activity-link={activity.garminActivityId}
            className="truncate font-semibold hover:underline rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            onClick={(event) => event.stopPropagation()}
          >
            {activity.title}
          </Link>
          <MetaList
            className="font-mono text-[11.5px] text-muted-foreground"
            items={[
              `${activity.startDateDisplay} ${activity.startClockDisplay}`,
              activity.hevyBadge && "via Hevy",
            ]}
          />
        </div>
      </td>
      <td className="px-2 py-3">
        <TypePill type={activity.activityType} />
      </td>
      <td className="px-2 py-3 font-mono text-[13px] whitespace-nowrap">{activity.detail.duration || "—"}</td>
      <td className="px-2 py-3 font-mono text-[13px] whitespace-nowrap">{activity.detail.distance || "—"}</td>
      <td className="hidden px-2 py-3 font-mono text-[13px] whitespace-nowrap lg:table-cell">{effort || "—"}</td>
      <td className="px-2 py-3">
        {/* One line, truncating rather than wrapping, so every row keeps the
            same height whichever pill it shows. */}
        <div className="grid grid-cols-[auto_minmax(0,max-content)] items-center gap-2">
          <StatusPill status={activity.publishStatus} publishing={publishing} reservePublishingWidth />
          {reason ? (
            // Hidden, not removed, while publishing so the row and column keep their size.
            <span
              title={reason}
              aria-hidden={publishing || undefined}
              className={cn(
                "max-w-52 truncate text-xs text-muted-foreground",
                publishing && "invisible",
              )}
            >
              {reason}
            </span>
          ) : null}
        </div>
      </td>
      <td className="px-2 py-3 text-center text-muted-foreground">
        <ChevronRightIcon className="inline size-4" aria-hidden="true" />
      </td>
    </tr>
  );
}
