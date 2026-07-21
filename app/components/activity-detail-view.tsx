import { ExternalLinkIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { activityDetailMetrics, isPublishableStatus } from "@/lib/activity-metrics";
import type { Activity } from "@/lib/api";

const publishDisabledReason = "Reconnect Strava to resume publishing.";

/**
 * Body content for the activity detail overlay's view mode (handoff frame
 * `2a`): stat grid, description, external links. Rendered inside
 * `ResponsiveOverlay`'s scrolling body — see `app/routes/activity-detail.tsx`
 * for how this is composed with `ActivityDetailViewFooter` and the edit-mode
 * counterpart in `activity-detail-edit.tsx`.
 *
 * The grid is 3 columns below `md:` and 4 from `md:` up, matching the
 * handoff's two frames exactly (mobile 2a is `repeat(3,1fr)`, desktop is
 * `repeat(4,1fr)`) rather than picking one column count for both.
 */
export function ActivityDetailView({ activity }: { activity: Activity }) {
  const metrics = activityDetailMetrics(activity);

  return (
    <div className="grid gap-6 md:gap-[26px]">
      {metrics.length > 0 ? (
        <dl className="grid grid-cols-3 gap-x-3 gap-y-4 md:grid-cols-4 md:gap-x-4 md:gap-y-[18px]">
          {metrics.map(([label, value]) => (
            <div key={label} className="flex flex-col gap-[3px]">
              <dt className="font-mono text-[10px] tracking-[0.08em] text-muted-foreground md:text-[10.5px]">
                {label}
              </dt>
              <dd className="font-mono text-[15px] font-semibold md:text-base">{value}</dd>
            </div>
          ))}
        </dl>
      ) : null}

      <Separator />

      <div className="grid gap-2">
        <h4 className="text-[13px] font-semibold text-foreground/90">Description</h4>
        <p className="text-sm leading-relaxed text-muted-foreground">
          {activity.description || "No description yet."}
        </p>
      </div>

      <div className="flex flex-wrap gap-2.5" aria-label="Activity links">
        <Button asChild variant="outline" size="sm">
          <a href={activity.garminUrl} target="_blank" rel="noreferrer">
            Garmin
            <ExternalLinkIcon data-icon="inline-end" />
          </a>
        </Button>
        {activity.stravaUrl ? (
          <Button asChild variant="outline" size="sm">
            <a href={activity.stravaUrl} target="_blank" rel="noreferrer">
              Strava
              <ExternalLinkIcon data-icon="inline-end" />
            </a>
          </Button>
        ) : null}
      </div>
    </div>
  );
}

export type ActivityDetailViewFooterProps = {
  activity: Activity;
  /** True while the Strava connection is broken — see `AppState.connections.broken`. */
  connectionsBroken: boolean;
  /** True while ANY activity action is in flight — blocks every button here. */
  busy: boolean;
  isPublishing: boolean;
  isExcluding: boolean;
  isRestoring: boolean;
  onEdit: () => void;
  onPublish: () => void;
  onExclude: () => void;
  onRestore: () => void;
};

/**
 * Footer for view mode: Publish full-width primary above a paired Edit +
 * Exclude row on mobile, Edit on the left with Exclude + Publish grouped on
 * the right on desktop (frame 2a). An `excluded` activity swaps Exclude for
 * Restore; a `published` one drops the primary action entirely (nothing left
 * to publish) — the handoff only shows the pending-activity state, so these
 * two variants are this task's own extrapolation of the same footer shape.
 *
 * Two parallel blocks toggled by CSS breakpoint (never JS), same convention
 * as `BulkActionBar` / `ActivitiesTable` / `ActivityCard` — both exist in the
 * DOM in tests; scope assertions with `within` or index into `getAllByRole`.
 */
export function ActivityDetailViewFooter({
  activity,
  connectionsBroken,
  busy,
  isPublishing,
  isExcluding,
  isRestoring,
  onEdit,
  onPublish,
  onExclude,
  onRestore,
}: ActivityDetailViewFooterProps) {
  const excluded = activity.publishStatus === "excluded";
  const publishable = isPublishableStatus(activity.publishStatus);

  const editButton = (className: string) => (
    <Button variant="outline" className={className} disabled={busy} onClick={onEdit}>
      Edit
    </Button>
  );

  const secondaryButton = (className: string) => {
    if (excluded) {
      return (
        <Button
          variant="outline"
          className={className}
          disabled={busy}
          aria-busy={isRestoring}
          onClick={onRestore}
        >
          {isRestoring ? (
            <>
              <Spinner data-icon="inline-start" aria-hidden="true" />
              Restoring…
            </>
          ) : (
            "Restore"
          )}
        </Button>
      );
    }
    if (publishable) {
      return (
        <Button
          variant="outline"
          className={className}
          disabled={busy}
          aria-busy={isExcluding}
          onClick={onExclude}
        >
          {isExcluding ? (
            <>
              <Spinner data-icon="inline-start" aria-hidden="true" />
              Excluding…
            </>
          ) : (
            "Exclude"
          )}
        </Button>
      );
    }
    return null;
  };

  const publishButton = (className: string) =>
    publishable ? (
      <Button
        className={className}
        disabled={busy || connectionsBroken}
        aria-busy={isPublishing}
        title={connectionsBroken ? publishDisabledReason : undefined}
        aria-description={connectionsBroken ? publishDisabledReason : undefined}
        onClick={onPublish}
      >
        {isPublishing ? (
          <>
            <Spinner data-icon="inline-start" aria-hidden="true" />
            Publishing…
          </>
        ) : activity.publishStatus === "missing" ? (
          "Republish"
        ) : (
          "Publish to Strava"
        )}
      </Button>
    ) : null;

  return (
    <>
      <div className="flex flex-col gap-2.5 md:hidden">
        {publishButton("w-full")}
        <div className="flex gap-2.5">
          {editButton("flex-1")}
          {secondaryButton("flex-1")}
        </div>
      </div>
      <div className="hidden items-center justify-between gap-2.5 md:flex">
        {editButton("")}
        <div className="flex gap-2.5">
          {secondaryButton("")}
          {publishButton("")}
        </div>
      </div>
    </>
  );
}
