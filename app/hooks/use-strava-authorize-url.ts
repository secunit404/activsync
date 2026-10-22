import { useQuery } from "@tanstack/react-query";

import { startStravaConnect } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

// Every fetch issues fresh OAuth state that replaces the previous one, so the
// URL is fetched once per mount and never cached past it or refetched.
const oncePerMount = {
  staleTime: Infinity,
  gcTime: 0,
  retry: false,
  refetchOnWindowFocus: false,
  refetchOnReconnect: false,
} as const;

export function useStravaConnectUrl(clientId: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.stravaConnectUrl(clientId),
    queryFn: ({ signal }) => startStravaConnect(signal),
    enabled,
    ...oncePerMount,
  });
}
