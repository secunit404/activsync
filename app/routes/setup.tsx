import { useQuery } from "@tanstack/react-query";
import { CircleAlertIcon, CheckIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Link, useSearchParams } from "react-router";

import { GarminStep } from "@/components/setup-garmin";
import { HevyStep } from "@/components/setup-hevy";
import { SetupStepDots } from "@/components/setup-step-dots";
import { StravaStep } from "@/components/setup-strava";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { useSettingsAction } from "@/hooks/use-settings-action";
import { cn } from "@/lib/utils";
import {
  cancelGarminMfa,
  completeGarminMfa,
  getSettings,
  runInitialSync,
  saveStravaCredentials,
  setupGarmin,
  setupHevy,
  skipHevy,
  type SettingsState,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { Route } from "./+types/setup";

export function meta(): Route.MetaDescriptors {
  return [{ title: "Set up ActivSync" }];
}

export default function Setup() {
  const [searchParams] = useSearchParams();
  const settings = useQuery({
    queryKey: queryKeys.settings,
    queryFn: ({ signal }) => getSettings(signal),
  });

  if (settings.isPending) {
    return (
      <WizardCard className="min-h-[420px] items-center justify-center gap-3 text-muted-foreground">
        <Spinner /> Loading setup…
      </WizardCard>
    );
  }
  if (settings.isError) {
    return (
      <WizardCard className="gap-5 p-7 md:p-10">
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Couldn’t load setup</AlertTitle>
          <AlertDescription>{settings.error.message}</AlertDescription>
        </Alert>
      </WizardCard>
    );
  }
  return (
    <SetupView
      state={settings.data}
      stravaError={searchParams.get("stravaError")}
    />
  );
}

export function SetupView({
  state,
  stravaError,
}: {
  state: SettingsState;
  stravaError?: string | null;
}) {
  const garmin = useSettingsAction(setupGarmin);
  const mfa = useSettingsAction(completeGarminMfa);
  const cancelMfa = useSettingsAction(cancelGarminMfa);
  const strava = useSettingsAction(saveStravaCredentials);
  const hevy = useSettingsAction(setupHevy);
  const skip = useSettingsAction(skipHevy);
  const initialSync = useSettingsAction(runInitialSync);
  const step = state.setup.step;

  if (state.setup.complete) {
    return (
      <WizardCard className="items-center justify-center gap-5 p-8 text-center md:p-10">
        <div className="grid size-12 place-items-center rounded-full bg-emerald-500/12 text-emerald-600">
          <CheckIcon className="size-6" />
        </div>
        <div className="grid gap-1.5">
          <h1 className="text-2xl font-extrabold tracking-[-0.02em]">
            You’re all set
          </h1>
          <p className="max-w-[40ch] text-sm text-muted-foreground">
            Garmin and Strava are connected and your activities are ready.
          </p>
        </div>
        <Button asChild size="lg" className="h-11">
          <Link to="/">Go to ActivSync</Link>
        </Button>
      </WizardCard>
    );
  }

  return (
    <WizardCard className="md:flex-row">
      <SetupStepDots step={step} />
      <main className="flex flex-1 flex-col gap-6 px-5 py-7 md:overflow-y-auto md:px-[46px] md:py-11">
        {stravaError ? (
          <Alert variant="destructive">
            <CircleAlertIcon />
            <AlertTitle>Strava connection failed</AlertTitle>
            <AlertDescription>{stravaError}</AlertDescription>
          </Alert>
        ) : null}
        {step === "garmin" ? (
          <GarminStep
            state={state}
            pending={garmin.isPending || mfa.isPending || cancelMfa.isPending}
            onConnect={(payload) => garmin.mutateAsync(payload)}
            onMfa={(code) => mfa.mutateAsync(code)}
            onCancelMfa={() => cancelMfa.mutateAsync()}
          />
        ) : null}
        {step === "strava" ? (
          <StravaStep
            state={state}
            pending={strava.isPending}
            onSave={async (payload) => {
              await strava.mutateAsync(payload);
              window.location.assign("/strava/connect");
            }}
          />
        ) : null}
        {step === "hevy" ? (
          <HevyStep
            pending={hevy.isPending || skip.isPending}
            onConnect={(apiKey) => hevy.mutateAsync(apiKey)}
            onSkip={() => skip.mutateAsync()}
          />
        ) : null}
        {step === "syncing" ? (
          <SyncStep
            pending={initialSync.isPending}
            onSync={() => initialSync.mutateAsync()}
          />
        ) : null}
      </main>
    </WizardCard>
  );
}

/** The boxed wizard card, frame `2d` — centered on a full-bleed dark page,
 * full-screen below `md:`. Loading/error/complete states reuse it without
 * the step rail; the active wizard adds `SetupStepDots` via `md:flex-row`. */
function WizardCard({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className="grid min-h-screen place-items-center bg-background px-0 py-0 md:px-6 md:py-10">
      <div
        className={cn(
          "flex w-full max-w-[1120px] flex-col border-border bg-card text-card-foreground md:min-h-[720px] md:rounded-2xl md:border md:shadow-[var(--shadow-frame)]",
          className,
        )}
      >
        {children}
      </div>
    </div>
  );
}

function SyncStep({
  pending,
  onSync,
}: {
  pending: boolean;
  onSync: () => Promise<unknown>;
}) {
  return (
    <div className="flex flex-1 flex-col gap-6 md:gap-7">
      <div className="flex flex-col gap-2">
        <p className="font-mono text-[11px] tracking-[0.2em] text-[var(--sync)] uppercase">
          Step 4 · Sync
        </p>
        <h2 className="text-2xl font-extrabold tracking-[-0.02em] md:text-[30px]">
          Ready for the first sync
        </h2>
        <p className="max-w-[52ch] text-sm leading-relaxed text-muted-foreground">
          ActivSync will fetch your selected history and keep every category
          held for review by default.
        </p>
      </div>
      <ul className="grid max-w-[440px] gap-3 text-sm">
        {["Garmin connected", "Strava connected", "Hevy choice saved"].map(
          (label) => (
            <li key={label} className="flex items-center gap-2">
              <CheckIcon className="size-4 text-emerald-600" /> {label}
            </li>
          ),
        )}
      </ul>
      <Button
        className="h-12 sm:w-fit"
        disabled={pending}
        onClick={() => void onSync()}
      >
        {pending ? <Spinner /> : null}
        {pending ? "Syncing activities…" : "Start initial sync"}
      </Button>
    </div>
  );
}
