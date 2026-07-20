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
