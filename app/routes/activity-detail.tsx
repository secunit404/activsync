import { useState } from "react";
import { data } from "react-router";
import { useNavigate, useOutletContext, useParams, useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  ActivityDetailEdit,
  ActivityDetailEditFooter,
} from "@/components/activity-detail-edit";
import { ActivityDetailView, ActivityDetailViewFooter } from "@/components/activity-detail-view";
import { ResponsiveOverlay } from "@/components/ui/responsive-overlay";
import { formatTypeLabel } from "@/components/type-pill";
import { useActivityActions } from "@/hooks/use-activity-actions";
import { getAppState, type Activity } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { ERROR_TOAST_DURATION_MS } from "@/lib/toast-duration";
import type { Route } from "./+types/activity-detail";

// `:id` (see app/routes.ts) has no built-in way to constrain its shape to
// "numeric" the way an Express-style `:id(\d+)` would — React Router route
// paths don't support inline regex. Without this guard, `:id` matches any
// segment, so a typo'd path like `/typo` would fall through to this
// placeholder instead of a 404. SPA mode (react-router.config.ts sets
// `ssr: false`) only supports `clientLoader`, not `loader`, on non-root
// routes — see the SPA guide.
export function clientLoader({ params }: Route.ClientLoaderArgs) {
  if (!/^\d+$/.test(params.id)) {
    throw data("Not Found", { status: 404, statusText: "Not Found" });
  }
  return null;
}

type ActionType = "publish" | "exclude" | "restore" | "edit";

/**
 * Activity detail — view (`2a`) and edit (`2b`), one `ResponsiveOverlay`
 * (`mobile="cover"`) whose `children`/`footer` swap between the two modes
 * rather than mounting two overlays (see Task 6's guidance: swapping avoids
 * a remount, which would lose focus and replay the enter animation).
 *
 * The activity list is read from `useOutletContext` — `ActivitiesView`
 * (the parent route) passes its already-fetched `data.items` down through
 * its own `<Outlet context={...} />` so this route never issues a second
 * fetch for data the list already has. `AppState` is fetched independently
 * via the same `queryKeys.appState` key `activities.tsx`/`app-layout.tsx`
 * already use — TanStack Query dedupes by key, so this is a cache read, not
 * a new request.
 */
export default function ActivityDetail() {
  const { id } = useParams();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const activities = useOutletContext<Activity[]>();
  const activity = activities.find(
    (item) => item.garminActivityId === Number(id),
  );
  const editing = searchParams.get("edit") === "1";
  const actions = useActivityActions();
  const appState = useQuery({
    queryKey: queryKeys.appState,
    queryFn: ({ signal }) => getAppState(signal),
  });
  const connectionsBroken = appState.data?.connections.broken.includes("strava") ?? false;

  // Re-seed the draft fields from the canonical activity whenever the
  // underlying activity changes or the user (re-)enters edit mode — Cancel
  // then Edit again must not resurrect a discarded draft, and a deep link
  // straight to `?edit=1` must start from the real saved values, not "".
  // Adjusted during render (React's documented escape hatch for "resetting
  // state when a prop changes") rather than in an Effect, which would cause
  // an extra commit-then-recommit render pass for no benefit here.
  const fieldsResetKey = activity ? `${activity.garminActivityId}:${editing}` : "";
  const [appliedResetKey, setAppliedResetKey] = useState(fieldsResetKey);
  const [title, setTitle] = useState(activity?.title ?? "");
  const [description, setDescription] = useState(activity?.description ?? "");
  if (activity && fieldsResetKey !== appliedResetKey) {
    setAppliedResetKey(fieldsResetKey);
    setTitle(activity.title);
    setDescription(activity.description);
  }

  const close = () => navigate("..");
  const handleOpenChange = (open: boolean) => {
    if (!open) {
      close();
    }
  };

  if (!activity) {
    // A stale/shared link can point at an id that isn't in the currently
    // loaded page (a different filter, a different page, or it simply
    // doesn't exist) — render a real explanatory overlay instead of a blank
    // one, per the brief's explicit "must not render a blank overlay".
    return (
      <ResponsiveOverlay
        open
        onOpenChange={handleOpenChange}
        title="Activity not found"
        mobile="cover"
      >
        <p className="text-sm text-muted-foreground">
          This activity isn&apos;t in the current list — it may be on a different page or filter.
          Close this and try again from the activity list.
        </p>
      </ResponsiveOverlay>
    );
  }

  const isPending = (type: ActionType) =>
    actions.isPending &&
    actions.variables?.type === type &&
    "activityId" in actions.variables &&
    actions.variables.activityId === activity.garminActivityId;

  const enterEdit = () => setSearchParams({ edit: "1" });
  const exitEdit = () => setSearchParams({}, { replace: true });

  const handlePublish = () => {
    actions.mutate({ type: "publish", activityId: activity.garminActivityId });
  };
  const handleExclude = () => {
    actions.mutate({ type: "exclude", activityId: activity.garminActivityId });
  };
  const handleRestore = () => {
    actions.mutate({ type: "restore", activityId: activity.garminActivityId });
  };

  // `useActivityActions` deliberately skips its own toast for "edit" (the
  // old inline dialog handled its own feedback) — see the hook's docstring.
  // This route fills that gap itself instead of adding a special case to a
  // hook shared by every other consumer.
  const handleSave = () => {
    actions.mutate(
      { type: "edit", activityId: activity.garminActivityId, title, description },
      {
        onSuccess: (result) => {
          toast.success(result.message);
          exitEdit();
        },
        onError: (error) => {
          toast.error(error instanceof Error ? error.message : "Could not save activity", {
            duration: ERROR_TOAST_DURATION_MS,
          });
        },
      },
    );
  };

  const dirty = title !== activity.title || description !== activity.description;
  const busy = actions.isPending;

  const eyebrow = editing
    ? `EDITING · ${activity.startDateDisplay} ${activity.startClockDisplay} · ${formatTypeLabel(activity.activityType)}`
    : `${activity.startDateDisplay} · ${activity.startClockDisplay} · ${formatTypeLabel(activity.activityType)}`;

  return (
    <ResponsiveOverlay
      open
      onOpenChange={handleOpenChange}
      title={activity.title}
      description={eyebrow}
      mobile="cover"
      footer={
        editing ? (
          <ActivityDetailEditFooter
            dirty={dirty}
            saving={isPending("edit")}
            onCancel={exitEdit}
            onSave={handleSave}
          />
        ) : (
          <ActivityDetailViewFooter
            activity={activity}
            connectionsBroken={connectionsBroken}
            busy={busy}
            isPublishing={isPending("publish")}
            isExcluding={isPending("exclude")}
            isRestoring={isPending("restore")}
            onEdit={enterEdit}
            onPublish={handlePublish}
            onExclude={handleExclude}
            onRestore={handleRestore}
          />
        )
      }
    >
      {editing ? (
        <ActivityDetailEdit
          activity={activity}
          title={title}
          description={description}
          onTitleChange={setTitle}
          onDescriptionChange={setDescription}
          disabled={isPending("edit")}
        />
      ) : (
        <ActivityDetailView activity={activity} />
      )}
    </ResponsiveOverlay>
  );
}
