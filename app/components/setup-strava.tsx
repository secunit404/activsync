import { useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { StravaConnectButton } from "@/components/strava-connect-button";
import { useStravaConnectUrl } from "@/hooks/use-strava-authorize-url";
import type { SettingsState } from "@/lib/api";

export type StravaStepProps = {
  state: SettingsState;
  pending: boolean;
  onSave: (payload: {
    clientId: string;
    clientSecret: string;
  }) => Promise<void>;
};

/** Step 3: Strava app credentials, then the OAuth handoff. */
export function StravaStep({ state, pending, onSave }: StravaStepProps) {
  const [clientId, setClientId] = useState(state.credentials.stravaClientId);
  const [clientSecret, setClientSecret] = useState("");
  const saved =
    Boolean(state.credentials.stravaClientId) && state.credentials.stravaClientSecretSaved;
  const trimmedClientId = clientId.trim();
  const trimmedSecret = clientSecret.trim();
  const dirty =
    trimmedClientId !== state.credentials.stravaClientId || Boolean(trimmedSecret);
  const authorize = useStravaConnectUrl(state.credentials.stravaClientId, saved);
  return (
    <div className="flex flex-1 flex-col gap-6 md:gap-7">
      <div className="flex flex-col gap-2">
        <p className="font-mono text-[11px] tracking-[0.2em] text-[var(--sync)] uppercase">
          Step 3: Strava
        </p>
        <h2 className="text-2xl font-extrabold tracking-[-0.02em] md:text-[30px]">
          Authorize Strava
        </h2>
        <p className="max-w-[52ch] text-sm leading-relaxed text-muted-foreground">
          Save your Strava API application credentials, then authorize
          ActivSync to publish on your behalf.
        </p>
      </div>
      <form
        className="grid max-w-[440px] gap-5"
        onSubmit={(event) => {
          event.preventDefault();
          void onSave({ clientId: trimmedClientId, clientSecret: trimmedSecret }).then(() =>
            setClientSecret(""),
          );
        }}
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="setup-strava-id">Strava client ID</FieldLabel>
            <Input
              id="setup-strava-id"
              className="h-12"
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
              className="h-12"
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
            Your Strava API application’s callback domain must match the
            address you use to open ActivSync.
          </AlertDescription>
        </Alert>
        <Button type="submit" className="h-12 sm:w-fit" disabled={pending}>
          {pending ? <Spinner /> : null}
          {pending ? "Saving…" : "Save credentials"}
        </Button>
        <StravaConnectButton
          href={saved && !dirty ? (authorize.data?.url ?? null) : null}
          error={authorize.error?.message}
        />
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
    </div>
  );
}
