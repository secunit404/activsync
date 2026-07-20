import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import type { SettingsActionResult } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function useSettingsAction<
  TVariables = void,
  TResult extends { message: string } = SettingsActionResult,
>(
  mutationFn: (variables: TVariables) => Promise<TResult>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: async (result) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.settings }),
        queryClient.invalidateQueries({ queryKey: queryKeys.appState }),
        queryClient.invalidateQueries({ queryKey: queryKeys.allActivities }),
        queryClient.invalidateQueries({ queryKey: queryKeys.hevyTools }),
        queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue }),
      ]);
      toast.success(result.message);
    },
    onError: (error) => {
      toast.error(error.message);
    },
  });
}
