import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { useAuthStatus } from "@/hooks/use-auth-status";
import { loginPath } from "@/lib/api";
import { getAuthStatus } from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";

const MAX_RETRY_DELAY_MS = 30_000;

/**
 * Subscribes to the server's refresh events while signed in. An EventSource
 * left alone retries a 401 forever, so on any error this closes the stream,
 * asks whether the session still exists, and either goes to /login or
 * reopens with backoff.
 */
export function useLiveRefresh() {
  const queryClient = useQueryClient();
  const status = useAuthStatus();
  const signedIn = status.data?.authenticated === true;

  useEffect(() => {
    if (!signedIn) return;
    let source: EventSource | null = null;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let attempt = 0;
    let stopped = false;

    const refreshQueries = () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.appState });
      void queryClient.invalidateQueries({ queryKey: queryKeys.allActivities });
      void queryClient.invalidateQueries({ queryKey: queryKeys.settings });
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue });
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyTools });
    };

    const handleError = async () => {
      source?.close();
      const stillSignedIn = await queryClient
        .fetchQuery({
          queryKey: queryKeys.authStatus,
          queryFn: ({ signal }) => getAuthStatus(signal),
          staleTime: 0,
        })
        .then(
          (latest) => latest.authenticated,
          () => true,
        );
      if (stopped) return;
      if (!stillSignedIn) {
        const { pathname, search } = window.location;
        window.location.assign(loginPath(`${pathname}${search}`));
        return;
      }
      const delay = Math.min(1000 * 2 ** attempt, MAX_RETRY_DELAY_MS);
      attempt += 1;
      retryTimer = setTimeout(connect, delay);
    };

    function connect() {
      source = new EventSource("/api/events");
      source.addEventListener("open", () => {
        attempt = 0;
      });
      source.addEventListener("refresh", refreshQueries);
      source.addEventListener("error", () => void handleError());
    }

    connect();
    return () => {
      stopped = true;
      clearTimeout(retryTimer);
      source?.close();
    };
  }, [queryClient, signedIn]);
}
