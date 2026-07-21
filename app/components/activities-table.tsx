import { ChevronRightIcon } from "lucide-react";
import { Link, useNavigate } from "react-router";

import { StatusPill } from "@/components/status-pill";
import { TypePill } from "@/components/type-pill";
import { Checkbox } from "@/components/ui/checkbox";
import { activityEffort } from "@/lib/activity-metrics";
import type { Activity } from "@/lib/api";
import { cn } from "@/lib/utils";

type ActivitiesTableProps = {
  activities: Activity[];
  selected: ReadonlySet<number>;
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
  onToggle,
  onToggleAll,
}: ActivitiesTableProps) {
  const allSelected = activities.length > 0 && selected.size === activities.length;
  const someSelected = selected.size > 0 && !allSelected;

  return (
    <div
      data-testid="activities-table"
      className="hidden overflow-hidden rounded-xl border border-border md:block"
    >
      <table className="w-full border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-border bg-card font-mono text-[10.5px] tracking-[0.1em] text-muted-foreground">
            <th scope="col" className="w-[38px] px-4 py-3">
              <Checkbox
                aria-label="Select all activities"
                checked={allSelected ? true : someSelected ? "indeterminate" : false}
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
  onToggle,
}: {
  activity: Activity;
  selected: boolean;
  onToggle: (id: number) => void;
}) {
  const navigate = useNavigate();
  const href = `/${activity.garminActivityId}`;
  const effort = activityEffort(activity);

  return (
    <tr
      className={cn(
        "cursor-pointer border-b border-border/60 last:border-b-0 hover:bg-muted/40",
        selected && "bg-primary/5",
      )}
      onClick={() => navigate(href)}
    >
      <td className="px-4 py-3" onClick={(event) => event.stopPropagation()}>
        <Checkbox
          aria-label={`Select ${activity.title}`}
          checked={selected}
          onCheckedChange={() => onToggle(activity.garminActivityId)}
        />
      </td>
      <td className="min-w-0 px-2 py-3">
        <div className="flex min-w-0 flex-col gap-0.5">
          <Link
            to={href}
            className="truncate font-semibold hover:underline"
            onClick={(event) => event.stopPropagation()}
          >
            {activity.title}
          </Link>
          <span className="font-mono text-[11.5px] text-muted-foreground">
            {activity.startDateDisplay} · {activity.startClockDisplay}
            {activity.hevyBadge ? " · via Hevy" : ""}
          </span>
        </div>
      </td>
      <td className="px-2 py-3">
        <TypePill type={activity.activityType} />
      </td>
      <td className="px-2 py-3 font-mono text-[13px]">{activity.detail.duration || "—"}</td>
      <td className="px-2 py-3 font-mono text-[13px]">{activity.detail.distance || "—"}</td>
      <td className="hidden px-2 py-3 font-mono text-[13px] lg:table-cell">{effort || "—"}</td>
      <td className="px-2 py-3">
        <StatusPill status={activity.publishStatus} />
      </td>
      <td className="px-2 py-3 text-center text-muted-foreground">
        <ChevronRightIcon className="inline size-4" aria-hidden="true" />
      </td>
    </tr>
  );
}
