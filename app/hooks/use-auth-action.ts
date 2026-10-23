import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { queryKeys } from "@/lib/query-keys";
import { ERROR_TOAST_DURATION_MS } from "@/lib/toast-duration";

const AUTH_QUERY_KEYS = [
  queryKeys.authStatus,
  queryKeys.authSettings,
  queryKeys.authSessions,
  queryKeys.oidcConfig,
];

export function useAuthAction<TVariables = void, TResult = unknown>(
  mutationFn: (variables: TVariables) => Promise<TResult>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: async (result) => {
      await Promise.all(
        AUTH_QUERY_KEYS.map((queryKey) => queryClient.invalidateQueries({ queryKey })),
      );
      if (result && typeof result === "object" && "message" in result) {
        toast.success(String(result.message));
      }
    },
    onError: (error) => {
      toast.error(error.message, { duration: ERROR_TOAST_DURATION_MS });
    },
  });
}
