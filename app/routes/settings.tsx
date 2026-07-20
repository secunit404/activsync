import { useQuery } from "@tanstack/react-query";
import { CircleAlertIcon } from "lucide-react";
import { Navigate, useSearchParams } from "react-router";

import { CategorySettings } from "@/components/settings-categories";
import { ConnectionsSettings } from "@/components/settings-connections";
import { HevySettings } from "@/components/settings-hevy";
import { PreferencesSettings } from "@/components/settings-preferences";
import { SettingsShell, SettingsSection } from "@/components/settings-shell";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Spinner } from "@/components/ui/spinner";
import { getSettings, type SettingsState } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { Route } from "./+types/settings";

export function meta(): Route.MetaDescriptors {
  return [
    { title: "Settings · ActivSync" },
    {
      name: "description",
      content: "Manage ActivSync connections, sync rules, and preferences.",
    },
  ];
}

export default function Settings() {
  const [searchParams] = useSearchParams();
  const settings = useQuery({
    queryKey: queryKeys.settings,
    queryFn: ({ signal }) => getSettings(signal),
  });

  if (settings.isPending) {
    return (
      <SettingsShell
        development={false}
        title="Settings"
        description="Loading your connections and sync rules…"
      >
        <SettingsSection title="Loading">
          <div className="flex items-center gap-2 text-muted-foreground">
            <Spinner /> Loading settings…
          </div>
        </SettingsSection>
      </SettingsShell>
    );
  }
  if (settings.isError) {
    return (
      <SettingsShell
        development={false}
        title="Settings"
        description="Manage your connections and sync rules."
      >
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Couldn’t load settings</AlertTitle>
          <AlertDescription>{settings.error.message}</AlertDescription>
        </Alert>
      </SettingsShell>
    );
  }
  if (!settings.data.setup.complete) {
    return <Navigate to="/setup" replace />;
  }
  return (
    <SettingsView
      state={settings.data}
      stravaError={searchParams.get("stravaError")}
    />
  );
}

export function SettingsView({
  state,
  stravaError,
}: {
  state: SettingsState;
  stravaError?: string | null;
}) {
  return (
    <SettingsShell
      development={state.development}
      version={state.version}
      update={state.update}
      title="Settings"
      description="Manage connections, decide what publishes automatically, and tune each sync leg."
    >
      {stravaError ? (
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Strava connection failed</AlertTitle>
          <AlertDescription>{stravaError}</AlertDescription>
        </Alert>
      ) : null}
      <div className="grid gap-6 lg:grid-cols-2 lg:items-start">
        <div className="grid gap-6">
          <ConnectionsSettings state={state} />
          <PreferencesSettings state={state} />
        </div>
        <div className="grid gap-6">
          <CategorySettings state={state} />
          <HevySettings state={state} />
        </div>
      </div>
    </SettingsShell>
  );
}
