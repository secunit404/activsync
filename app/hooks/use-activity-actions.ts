import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  editActivity,
  excludeActivity,
  restoreActivity,
  type Activity,
  type ActivityActionResult,
  type RestoreTarget,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { ERROR_TOAST_DURATION_MS, UNDO_TOAST_DURATION_MS } from "@/lib/toast-duration";

/** What an exclude needs to know to be undone: the row and where it was. */
export type ExcludeTarget = Pick<Activity, "garminActivityId" | "publishStatus" | "holdReason">;

export type ActivityActionInput =
  | { type: "exclude"; activity: ExcludeTarget }
  | { type: "exclude-many"; activities: ExcludeTarget[] }
  | { type: "restore"; activityId: number }
  | {
      type: "edit";
      activityId: number;
      title: string;
      description: string;
    };

/** An exclude also reports which rows it actually excluded, for Undo. */
export type ActivityActionOutcome = ActivityActionResult & { excluded?: ExcludeTarget[] };

export type ActivityActionState = {
  isPending: boolean;
  action: ActivityActionInput | undefined;
};

export function useActivityActions() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: runActivityAction,
    onSuccess: (result, action) => {
      if (result.excluded?.length) {
        toastExcluded(queryClient, result, result.excluded);
      } else if (action.type !== "edit") {
        toastActionResult(result);
      }
    },
    onError: (error, action) => {
      if (action.type !== "edit") {
        toastActionError(error);
      }
    },
    onSettled: () => invalidateActivityQueries(queryClient),
  });
}

export function toastActionResult(result: ActivityActionResult) {
  if (result.severity === "warning") {
    toast.warning(result.message);
  } else {
    toast.success(result.message);
  }
}

export function toastActionError(error: Error) {
  toast.error(error.message, { duration: ERROR_TOAST_DURATION_MS });
}

export async function invalidateActivityQueries(queryClient: QueryClient) {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.appState }),
    queryClient.invalidateQueries({ queryKey: queryKeys.allActivities }),
  ]);
}

/**
 * Exclude is applied on the click, never deferred: a pending activity held
 * back only in this tab would still publish if the owner left. Undo instead
 * puts each row back where it was, so an undone backlog hold or missing row
 * cannot start publishing on its own.
 */
function toastExcluded(
  queryClient: QueryClient,
  result: ActivityActionResult,
  excluded: ExcludeTarget[],
) {
  const show = result.severity === "warning" ? toast.warning : toast;
  show(result.message, {
    duration: UNDO_TOAST_DURATION_MS,
    action: { label: "Undo", onClick: () => void undoExclude(queryClient, excluded) },
  });
}

async function undoExclude(queryClient: QueryClient, excluded: ExcludeTarget[]) {
  const results = await Promise.allSettled(
    excluded.map((activity) =>
      restoreActivity(activity.garminActivityId, restoreTargetOf(activity)),
    ),
  );
  const failed = results.filter((result) => result.status === "rejected").length;
  const restored = results.length - failed;

  if (failed === 0) {
    const noun = restored === 1 ? "activity" : "activities";
    toast(`Restored ${restored} ${noun}`, { description: "Back where they were." });
  } else {
    toast.error(
      restored === 0
        ? "Could not undo the exclude"
        : `Restored ${restored}; ${failed} stayed excluded`,
      { duration: ERROR_TOAST_DURATION_MS },
    );
  }
  await invalidateActivityQueries(queryClient);
}

function restoreTargetOf(activity: ExcludeTarget): RestoreTarget {
  const { publishStatus, holdReason } = activity;
  if (publishStatus === "pending" || publishStatus === "missing") {
    return { status: publishStatus, holdReason: null };
  }
  return { status: "held", holdReason };
}

async function runActivityAction(action: ActivityActionInput): Promise<ActivityActionOutcome> {
  switch (action.type) {
    case "exclude": {
      const result = await excludeActivity(action.activity.garminActivityId);
      return { ...result, excluded: [action.activity] };
    }
    case "exclude-many":
      return excludeMany(action.activities);
    case "restore":
      return restoreActivity(action.activityId);
    case "edit":
      return editActivity(
        action.activityId,
        action.title,
        action.description,
      );
  }
}

/**
 * There is no bulk-exclude endpoint (unlike publish), so this fires one
 * `excludeActivity` call per id and folds the settled results into a single
 * outcome — one toast and one query invalidation, the same shape the
 * bulk-publish endpoint already returns for its own partial failures.
 * Resolves (never rejects) unless every call failed, so a caller that clears
 * the selection `onSuccess` also clears it on a partial failure, keeping only
 * a total failure selected for retry.
 */
async function excludeMany(activities: ExcludeTarget[]): Promise<ActivityActionOutcome> {
  const results = await Promise.allSettled(
    activities.map((activity) => excludeActivity(activity.garminActivityId)),
  );
  const excluded = activities.filter((_, index) => results[index].status === "fulfilled");
  const succeeded = excluded.length;
  const failed = results.length - succeeded;

  if (failed === 0) {
    const noun = succeeded === 1 ? "activity" : "activities";
    return {
      message: `Excluded ${succeeded} ${noun}`,
      severity: "success",
      publishedCount: 0,
      failedCount: 0,
      blockedCount: 0,
      excluded,
    };
  }

  if (succeeded === 0) {
    throw new Error(
      failed === 1
        ? "Could not exclude the selected activity"
        : `Could not exclude ${failed} activities`,
    );
  }

  return {
    message: `Excluded ${succeeded}; ${failed} failed and stayed as-is`,
    severity: "warning",
    publishedCount: 0,
    failedCount: failed,
    blockedCount: 0,
    excluded,
  };
}
