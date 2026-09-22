import { CircleCheckIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import type { AppState } from "@/lib/api";

type CatchUpReportProps = {
  report: AppState["catchUpReport"];
  onDismiss: () => void;
};

/**
 * "Catch-up report" (handoff frame `3b`) — shown once ActivSync has
 * reconciled the backlog after a Garmin/Strava outage. Desktop only per the
 * handoff (the mobile `3b` reference sheet omits this card entirely), so it
 * is hidden below `md:` rather than gated in the caller. Informational, not
 * `role="alert"` — nothing here needs interrupting the user, it is a summary
 * of what already happened while a service was down.
 *
 * Renders nothing once `report` is `null` (dismissed, or nothing to report)
 * so callers can mount it unconditionally.
 */
export function CatchUpReport({ report, onDismiss }: CatchUpReportProps) {
  if (!report) {
    return null;
  }

  const activityNoun = report.new === 1 ? "activity" : "activities";
  const dayNoun = report.days === 1 ? "day" : "days";

  return (
    <div className="hidden items-start gap-3 rounded-xl border border-border bg-card p-4 md:flex">
      <span
        aria-hidden="true"
        className="grid size-[22px] shrink-0 place-items-center rounded-full bg-success/15 text-success"
      >
        <CircleCheckIcon className="size-3.5" />
      </span>
      <div className="grid flex-1 gap-0.5">
        <p className="text-sm font-bold">Reconnect catch-up complete</p>
        <p className="text-[13px] leading-relaxed text-muted-foreground">
          Found {report.new} {activityNoun} over the last {report.days} {dayNoun} —{" "}
          {report.linked} already existed on Strava and {report.held} are held for review.
        </p>
      </div>
      <Button variant="outline" className="h-9 shrink-0" onClick={onDismiss}>
        Dismiss
      </Button>
    </div>
  );
}
