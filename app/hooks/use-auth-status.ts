import { useQuery } from "@tanstack/react-query";

import { getAuthStatus } from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";

export function useAuthStatus() {
  return useQuery({
    queryKey: queryKeys.authStatus,
    queryFn: ({ signal }) => getAuthStatus(signal),
    staleTime: 0,
  });
}
