import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  editActivity,
  excludeActivity,
  publishActivities,
  publishActivity,
  restoreActivity,
  type ActivityActionResult,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export type ActivityActionInput =
  | { type: "publish"; activityId: number }
  | { type: "publish-many"; activityIds: number[] }
  | { type: "exclude"; activityId: number }
  | { type: "exclude-many"; activityIds: number[] }
  | { type: "restore"; activityId: number }
  | {
      type: "edit";
      activityId: number;
      title: string;
      description: string;
    };

export type ActivityActionState = {
  isPending: boolean;
  action: ActivityActionInput | undefined;
};

export function useActivityActions() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: runActivityAction,
    onSuccess: (result, action) => {
      if (action.type === "edit") {
        return;
      }
      if (result.severity === "warning") {
        toast.warning(result.message);
      } else {
        toast.success(result.message);
      }
    },
    onError: (error, action) => {
      if (action.type !== "edit") {
        toast.error(error.message);
      }
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.appState }),
        queryClient.invalidateQueries({ queryKey: queryKeys.allActivities }),
      ]);
    },
  });
}

function runActivityAction(
  action: ActivityActionInput,
): Promise<ActivityActionResult> {
  switch (action.type) {
    case "publish":
      return publishActivity(action.activityId);
    case "publish-many":
      return publishActivities(action.activityIds);
    case "exclude":
      return excludeActivity(action.activityId);
    case "exclude-many":
      return excludeMany(action.activityIds);
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
 * `ActivityActionResult` — one toast and one query invalidation, the same
 * shape the bulk-publish endpoint already returns for its own partial
 * failures. Resolves (never rejects) unless every call failed, so a caller
 * that clears the selection `onSuccess` also clears it on a partial
 * failure, keeping only a total failure selected for retry.
 */
async function excludeMany(activityIds: number[]): Promise<ActivityActionResult> {
  const results = await Promise.allSettled(activityIds.map((id) => excludeActivity(id)));
  const succeeded = results.filter((result) => result.status === "fulfilled").length;
  const failed = results.length - succeeded;

  if (failed === 0) {
    const noun = succeeded === 1 ? "activity" : "activities";
    return {
      message: `Excluded ${succeeded} ${noun}`,
      severity: "success",
      publishedCount: 0,
      failedCount: 0,
      blockedCount: 0,
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
  };
}
