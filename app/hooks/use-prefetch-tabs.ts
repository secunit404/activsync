import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { getHevyQueue, getHevyTools, getSettings } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

/**
 * Warm the caches for the tabs the user has not opened yet.
 *
 * Each tab renders a loading state while its first request is in flight, and
 * on a local backend that request finishes in about 11ms — long enough for the
 * spinner to paint for a single frame and vanish, which reads as a flicker on
 * the first visit after a reload and never again once the data is cached. The
 * cure is not a nicer loading state but not needing one: fetch the two other
 * tabs' data up front, so tapping a tab has it already.
 *
 * Activities is left out — it is the landing route and fetches its own data
 * immediately. `prefetchQuery` respects the client's `staleTime` (15s, set in
 * root.tsx) and resolves without throwing, so a backend that is down here
 * costs nothing: the route itself will surface the error when it is opened.
 */
export function usePrefetchTabs() {
  const queryClient = useQueryClient();

  useEffect(() => {
    void queryClient.prefetchQuery({
      queryKey: queryKeys.settings,
      queryFn: ({ signal }) => getSettings(signal),
    });
    void queryClient.prefetchQuery({
      queryKey: queryKeys.hevyQueue,
      queryFn: ({ signal }) => getHevyQueue(signal),
    });
    void queryClient.prefetchQuery({
      queryKey: queryKeys.hevyTools,
      queryFn: ({ signal }) => getHevyTools(signal),
    });
  }, [queryClient]);
}
