import type { ActivityQuery } from "@/lib/api";

export const queryKeys = {
  appState: ["app-state"] as const,
  authStatus: ["auth-status"] as const,
  authSettings: ["auth-settings"] as const,
  authSessions: ["auth-sessions"] as const,
  oidcConfig: ["oidc-config"] as const,
  settings: ["settings"] as const,
  stravaConnectUrl: (clientId: string) => ["strava-connect-url", clientId] as const,
  stravaLoginUrl: (next: string) => ["strava-login-url", next] as const,
  hevyTools: ["hevy-tools"] as const,
  hevyDeviceOptions: ["hevy-device-options"] as const,
  hevyQueue: ["hevy-queue"] as const,
  hevyWorkout: (hevyId: string) => ["hevy-workout", hevyId] as const,
  activities: (query: ActivityQuery) => ["activities", query] as const,
  allActivities: ["activities"] as const,
};
