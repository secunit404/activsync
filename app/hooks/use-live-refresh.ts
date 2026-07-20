import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { queryKeys } from "@/lib/query-keys";

export function useLiveRefresh() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const source = new EventSource("/api/events");
    const refreshQueries = () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.appState });
      void queryClient.invalidateQueries({ queryKey: queryKeys.allActivities });
      void queryClient.invalidateQueries({ queryKey: queryKeys.settings });
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue });
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyTools });
    };

    source.addEventListener("refresh", refreshQueries);
    return () => {
      source.removeEventListener("refresh", refreshQueries);
      source.close();
    };
  }, [queryClient]);
}
