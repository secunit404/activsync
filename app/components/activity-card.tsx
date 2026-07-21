import { Link, useNavigate } from "react-router";

import { formatTypeLabel } from "@/components/type-pill";
import { StatusPill } from "@/components/status-pill";
import { Checkbox } from "@/components/ui/checkbox";
import { activityEffort } from "@/lib/activity-metrics";
import type { Activity } from "@/lib/api";
import { cn } from "@/lib/utils";

type ActivityCardProps = {
  activity: Activity;
  selected: boolean;
  onToggle: (id: number) => void;
};

/**
 * Full-width mobile list card for the Activities screen (handoff frame
 * `1a`, mobile). The checkbox is always visible — never hover-revealed, per
 * the handoff. Detail/edit content that used to live in this file moved to
 * Task 11's overlay.
 *
 * Mirrors `ActivitiesTable`'s row pattern rather than giving the whole card
 * `role="link"`: that role computes its accessible name from ALL descendant
 * text (title + status + metadata + stats), which collided with unrelated
 * page links containing the same substring (e.g. a Hevy-sourced card's "via
 * Hevy" text made it match `getByRole("link", { name: "Hevy" })` elsewhere
 * on the page). Instead the card <div> has a plain onClick for pointer
 * users, and the title is a real `<Link>` for keyboard/screen-reader access
 * — same division of labor as the table row.
 */
export function ActivityCard({ activity, selected, onToggle }: ActivityCardProps) {
  const navigate = useNavigate();
  const href = `/${activity.garminActivityId}`;
  const source = activity.hevyBadge ? "via Hevy" : formatTypeLabel(activity.activityType);
  const statLine = [activity.detail.duration, activity.detail.distance, activityEffort(activity)]
    .filter(Boolean)
    .join(" · ");

  return (
    <div
      onClick={() => navigate(href)}
      className={cn(
        "flex cursor-pointer items-center gap-3 rounded-xl border border-border p-3.5",
        selected && "border-primary/30 bg-primary/5",
      )}
    >
      <Checkbox
        aria-label={`Select ${activity.title}`}
        checked={selected}
        onClick={(event) => event.stopPropagation()}
        onCheckedChange={() => onToggle(activity.garminActivityId)}
      />
      <div className="grid min-w-0 flex-1 gap-1">
        <div className="flex items-center justify-between gap-2">
          <Link
            to={href}
            onClick={(event) => event.stopPropagation()}
            className="truncate text-[15px] font-bold hover:underline"
          >
            {activity.title}
          </Link>
          <StatusPill status={activity.publishStatus} />
        </div>
        <span className="font-mono text-[11.5px] text-muted-foreground">
          {activity.startDateDisplay} · {activity.startClockDisplay} · {source}
        </span>
        {statLine ? (
          <span className="font-mono text-xs text-foreground/80">{statLine}</span>
        ) : null}
      </div>
    </div>
  );
}
