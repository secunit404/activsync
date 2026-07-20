import { useQuery } from "@tanstack/react-query";
import { CircleAlertIcon, CheckIcon } from "lucide-react";
import { useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router";

import { SettingsShell, SettingsSection } from "@/components/settings-shell";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useSettingsAction } from "@/hooks/use-settings-action";
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
  type SetupStep,
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
    return <SetupLoading />;
  }
  if (settings.isError) {
    return (
      <SettingsShell
        development={false}
        title="Set up ActivSync"
        description="Connect the services ActivSync needs."
      >
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Couldn’t load setup</AlertTitle>
          <AlertDescription>{settings.error.message}</AlertDescription>
        </Alert>
      </SettingsShell>
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
      <SettingsShell
        development={state.development}
        version={state.version}
        update={state.update}
        title="You’re all set"
        description="Garmin and Strava are connected and your activities are ready."
      >
        <SettingsSection title="Setup complete">
          <div className="grid gap-4 text-center sm:justify-items-start sm:text-left">
            <div className="mx-auto grid size-12 place-items-center rounded-full bg-emerald-500/12 text-emerald-600 sm:mx-0">
              <CheckIcon className="size-6" />
            </div>
            <Button asChild size="lg" className="h-11">
              <Link to="/">Go to ActivSync</Link>
            </Button>
          </div>
        </SettingsSection>
      </SettingsShell>
    );
  }

  return (
    <SettingsShell
      development={state.development}
      version={state.version}
      update={state.update}
      title="Set up ActivSync"
      description="Four short steps. Credentials stay in your local ActivSync database."
    >
      {stravaError ? (
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Strava connection failed</AlertTitle>
          <AlertDescription>{stravaError}</AlertDescription>
        </Alert>
      ) : null}
      <SetupProgress step={step} />
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
    </SettingsShell>
  );
}

function SetupProgress({ step }: { step: SetupStep | null }) {
  const steps: Array<{ key: SetupStep; label: string }> = [
    { key: "garmin", label: "Garmin" },
    { key: "strava", label: "Strava" },
    { key: "hevy", label: "Hevy" },
    { key: "syncing", label: "Sync" },
  ];
  const activeIndex = Math.max(
    steps.findIndex((item) => item.key === step),
    0,
  );
  return (
    <ol className="grid grid-cols-4 gap-2" aria-label="Setup progress">
      {steps.map((item, index) => (
        <li key={item.key} className="grid gap-1.5 text-center">
          <span
            className={
              "h-1.5 rounded-full " +
              (index <= activeIndex ? "bg-primary" : "bg-muted")
            }
          />
          <span
            className={
              "text-xs " +
              (index === activeIndex ? "font-semibold" : "text-muted-foreground")
            }
          >
            {item.label}
          </span>
        </li>
      ))}
    </ol>
  );
}

function GarminStep({
  state,
  pending,
  onConnect,
  onMfa,
  onCancelMfa,
}: {
  state: SettingsState;
  pending: boolean;
  onConnect: (payload: {
    email: string;
    password: string;
    lookbackDays: number;
    detectedTimezone: string;
  }) => Promise<unknown>;
  onMfa: (code: string) => Promise<unknown>;
  onCancelMfa: () => Promise<unknown>;
}) {
  const [email, setEmail] = useState(state.credentials.garminEmail);
  const [password, setPassword] = useState("");
  const [lookbackDays, setLookbackDays] = useState(
    state.preferences.lookbackDays,
  );
  const [code, setCode] = useState("");

  if (state.setup.mfaRequired) {
    return (
      <SettingsSection
        title="Verify Garmin"
        description="Garmin sent a verification code. Enter it to finish connecting."
      >
        <form
          className="grid gap-5"
          onSubmit={(event) => {
            event.preventDefault();
            void onMfa(code);
          }}
        >
          <Field>
            <FieldLabel htmlFor="setup-mfa">Verification code</FieldLabel>
            <Input
              id="setup-mfa"
              className="h-11"
              inputMode="numeric"
              autoComplete="one-time-code"
              value={code}
              onChange={(event) => setCode(event.target.value)}
              required
              autoFocus
            />
          </Field>
          <div className="flex flex-col gap-2 sm:flex-row">
            <SubmitButton pending={pending} label="Verify code" busy="Verifying…" />
            <Button
              type="button"
              variant="outline"
              className="h-11"
              disabled={pending}
              onClick={() => void onCancelMfa()}
            >
              Cancel
            </Button>
          </div>
        </form>
      </SettingsSection>
    );
  }

  return (
    <SettingsSection
      title="Connect Garmin"
      description="ActivSync reads your Garmin activities and keeps them held until you review them."
    >
      <form
        className="grid gap-5"
        onSubmit={(event: FormEvent) => {
          event.preventDefault();
          void onConnect({
            email,
            password,
            lookbackDays,
            detectedTimezone:
              Intl.DateTimeFormat().resolvedOptions().timeZone ?? "",
          });
        }}
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="setup-garmin-email">Garmin email</FieldLabel>
            <Input
              id="setup-garmin-email"
              className="h-11"
              type="email"
              autoComplete="username"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              required
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="setup-garmin-password">Garmin password</FieldLabel>
            <Input
              id="setup-garmin-password"
              className="h-11"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              required={!state.credentials.garminPasswordSaved}
              placeholder={
                state.credentials.garminPasswordSaved
                  ? "Saved — leave blank to keep it"
                  : undefined
              }
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="setup-lookback">
              Activity history window (days)
            </FieldLabel>
            <Input
              id="setup-lookback"
              className="h-11"
              type="number"
              min={1}
              value={lookbackDays}
              onChange={(event) => setLookbackDays(event.target.valueAsNumber)}
              required
            />
            <FieldDescription>
              How far back the first Garmin sync should search.
            </FieldDescription>
          </Field>
        </FieldGroup>
        <SubmitButton pending={pending} label="Connect Garmin" busy="Connecting…" />
      </form>
    </SettingsSection>
  );
}

function StravaStep({
  state,
  pending,
  onSave,
}: {
  state: SettingsState;
  pending: boolean;
  onSave: (payload: { clientId: string; clientSecret: string }) => Promise<void>;
}) {
  const [clientId, setClientId] = useState(state.credentials.stravaClientId);
  const [clientSecret, setClientSecret] = useState("");
  return (
    <SettingsSection
      title="Connect Strava"
      description="Save your Strava API application credentials, then authorize ActivSync."
    >
      <form
        className="grid gap-5"
        onSubmit={(event) => {
          event.preventDefault();
          void onSave({ clientId, clientSecret });
        }}
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="setup-strava-id">Strava client ID</FieldLabel>
            <Input
              id="setup-strava-id"
              className="h-11"
              value={clientId}
              onChange={(event) => setClientId(event.target.value)}
              required
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="setup-strava-secret">
              Strava client secret
            </FieldLabel>
            <Input
              id="setup-strava-secret"
              className="h-11"
              type="password"
              autoComplete="new-password"
              value={clientSecret}
              onChange={(event) => setClientSecret(event.target.value)}
              required={!state.credentials.stravaClientSecretSaved}
              placeholder={
                state.credentials.stravaClientSecretSaved
                  ? "Saved — leave blank to keep it"
                  : undefined
              }
            />
          </Field>
        </FieldGroup>
        <Alert>
          <AlertTitle>Authorization callback</AlertTitle>
          <AlertDescription>
            Your Strava API application’s callback domain must match the address
            you use to open ActivSync.
          </AlertDescription>
        </Alert>
        <SubmitButton pending={pending} label="Save and authorize" busy="Saving…" />
        <p className="text-sm text-muted-foreground">
          Need the keys? See{" "}
          <a
            href="https://developers.strava.com/docs/getting-started/"
            target="_blank"
            rel="noreferrer"
          >
            Strava’s setup guide
          </a>
          .
        </p>
      </form>
    </SettingsSection>
  );
}

function HevyStep({
  pending,
  onConnect,
  onSkip,
}: {
  pending: boolean;
  onConnect: (apiKey: string) => Promise<unknown>;
  onSkip: () => Promise<unknown>;
}) {
  const [apiKey, setApiKey] = useState("");
  return (
    <SettingsSection
      title="Connect Hevy (optional)"
      description="Sync Hevy strength workouts onto Garmin and onward to Strava. Hevy Pro is required."
    >
      <form
        className="grid gap-5"
        onSubmit={(event) => {
          event.preventDefault();
          void onConnect(apiKey);
        }}
      >
        <Field>
          <FieldLabel htmlFor="setup-hevy-key">Hevy API key</FieldLabel>
          <Input
            id="setup-hevy-key"
            className="h-11"
            type="password"
            value={apiKey}
            onChange={(event) => setApiKey(event.target.value)}
            required
          />
          <FieldDescription>
            Disable any standalone hevy2garmin instance before enabling this.
          </FieldDescription>
        </Field>
        <div className="flex flex-col gap-2 sm:flex-row">
          <SubmitButton pending={pending} label="Connect Hevy" busy="Validating…" />
          <Button
            type="button"
            variant="outline"
            className="h-11"
            disabled={pending}
            onClick={() => void onSkip()}
          >
            Skip for now
          </Button>
        </div>
      </form>
    </SettingsSection>
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
    <SettingsSection
      title="Ready for the first sync"
      description="ActivSync will fetch your selected history and keep every category held for review by default."
    >
      <div className="grid gap-5">
        <ul className="grid gap-3 text-sm">
          {[
            "Garmin connected",
            "Strava connected",
            "Hevy choice saved",
          ].map((label) => (
            <li key={label} className="flex items-center gap-2">
              <CheckIcon className="size-4 text-emerald-600" /> {label}
            </li>
          ))}
        </ul>
        <Button
          className="h-11 sm:w-fit"
          disabled={pending}
          onClick={() => void onSync()}
        >
          {pending ? <Spinner /> : null}
          {pending ? "Syncing activities…" : "Start initial sync"}
        </Button>
      </div>
    </SettingsSection>
  );
}

function SubmitButton({
  pending,
  label,
  busy,
}: {
  pending: boolean;
  label: string;
  busy: string;
}) {
  return (
    <Button type="submit" className="h-11 sm:w-fit" disabled={pending}>
      {pending ? <Spinner /> : null}
      {pending ? busy : label}
    </Button>
  );
}

function SetupLoading() {
  return (
    <SettingsShell
      development={false}
      title="Set up ActivSync"
      description="Loading your setup progress…"
    >
      <SettingsSection title="Loading">
        <div className="flex items-center gap-2 text-muted-foreground">
          <Spinner /> Loading setup…
        </div>
      </SettingsSection>
    </SettingsShell>
  );
}
