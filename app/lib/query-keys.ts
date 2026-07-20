import type { ActivityQuery } from "@/lib/api";

export const queryKeys = {
  appState: ["app-state"] as const,
  settings: ["settings"] as const,
  hevyTools: ["hevy-tools"] as const,
  hevyQueue: ["hevy-queue"] as const,
  activities: (query: ActivityQuery) => ["activities", query] as const,
  allActivities: ["activities"] as const,
};
