import { useMutation, useQueryClient } from "@tanstack/react-query";

import { saveStravaCredentials, startStravaConnect } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export type StravaCredentials = { clientId: string; clientSecret: string };

/**
 * Saves changed credentials, then leaves for Strava's consent page. The
 * authorize URL is fetched on the click, never ahead of it: every fetch issues
 * fresh OAuth state that replaces the previous one.
 */
export function useStravaConnect() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (changed: StravaCredentials | null) => {
      if (changed) {
        await saveStravaCredentials(changed);
        void queryClient.invalidateQueries({ queryKey: queryKeys.settings });
      }
      const { url } = await startStravaConnect();
      window.location.assign(url);
    },
  });
}
