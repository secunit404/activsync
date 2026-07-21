import { useState } from "react";

import { ConnectionStatus, SettingsSection } from "@/components/settings-shell";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
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
  disconnectHevy,
  disconnectStrava,
  reconnectGarmin,
  runManualSync,
  saveHevyCredentials,
  saveStravaCredentials,
  type SettingsState,
} from "@/lib/api";

export function ConnectionsSettings({ state }: { state: SettingsState }) {
  const syncGarmin = useSettingsAction(() => runManualSync("garmin"));
  const syncStrava = useSettingsAction(() => runManualSync("strava"));
  return (
    <SettingsSection
      id="connections"
      title="Connections"
      description="Credentials are stored locally. Reconnecting only replaces them after verification succeeds."
    >
      <div>
        <ConnectionStatus
          name="Garmin Connect"
          connected={state.connections.garmin.connected}
          status={state.connections.garmin.status}
          meta={state.connections.garmin.meta}
          action={<GarminDialog state={state} />}
        />
        <ConnectionStatus
          name="Strava"
          connected={state.connections.strava.connected}
          status={state.connections.strava.status}
          meta={state.connections.strava.meta}
          action={<StravaDialog state={state} />}
        />
        <ConnectionStatus
          name="Hevy"
          connected={state.hevy.connected}
          status={state.hevy.status}
          action={<HevyDialog state={state} />}
        />
      </div>
      <div className="mt-5 grid gap-3 border-t border-border/70 pt-5">
        <div>
          <h3 className="font-semibold">Manual sync</h3>
          <p className="text-sm text-muted-foreground">
            ActivSync polls automatically. Use these to check right now.
          </p>
        </div>
        <div className="grid grid-cols-2 gap-2 sm:flex">
          <Button
            variant="outline"
            className="h-11"
            disabled={
              !state.connections.garmin.connected || syncGarmin.isPending
            }
            onClick={() => syncGarmin.mutate()}
          >
            {syncGarmin.isPending ? <Spinner /> : null}
            {syncGarmin.isPending ? "Syncing…" : "Sync Garmin"}
          </Button>
          <Button
            variant="outline"
            className="h-11"
            disabled={
              !state.connections.strava.connected || syncStrava.isPending
            }
            onClick={() => syncStrava.mutate()}
          >
            {syncStrava.isPending ? <Spinner /> : null}
            {syncStrava.isPending ? "Syncing…" : "Sync Strava"}
          </Button>
        </div>
      </div>
    </SettingsSection>
  );
}

function GarminDialog({ state }: { state: SettingsState }) {
  const [open, setOpen] = useState(state.setup.mfaRequired);
  const [email, setEmail] = useState(state.credentials.garminEmail);
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const reconnect = useSettingsAction(reconnectGarmin);
  const mfa = useSettingsAction(completeGarminMfa);
  const cancelMfa = useSettingsAction(cancelGarminMfa);
  const pending = reconnect.isPending || mfa.isPending || cancelMfa.isPending;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="outline" className="h-10">
          {state.connections.garmin.connected ? "Manage" : "Reconnect"}
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[calc(100vh-2rem)] overflow-y-auto sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Manage Garmin</DialogTitle>
          <DialogDescription>
            {state.setup.mfaRequired
              ? "Enter the verification code Garmin sent."
              : "Leave the password blank to keep the saved one."}
          </DialogDescription>
        </DialogHeader>
        {state.setup.mfaRequired ? (
          <form
            className="grid gap-4"
            onSubmit={async (event) => {
              event.preventDefault();
              await mfa.mutateAsync(code);
              setOpen(false);
            }}
          >
            <Field>
              <FieldLabel htmlFor="settings-garmin-mfa">
                Verification code
              </FieldLabel>
              <Input
                id="settings-garmin-mfa"
                className="h-11"
                inputMode="numeric"
                autoComplete="one-time-code"
                value={code}
                onChange={(event) => setCode(event.target.value)}
                required
                autoFocus
              />
            </Field>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                className="h-11"
                disabled={pending}
                onClick={async () => {
                  await cancelMfa.mutateAsync();
                  setOpen(false);
                }}
              >
                Cancel verification
              </Button>
              <Button type="submit" className="h-11" disabled={pending}>
                {mfa.isPending ? <Spinner /> : null}
                {mfa.isPending ? "Verifying…" : "Verify code"}
              </Button>
            </DialogFooter>
          </form>
        ) : (
          <form
            className="grid gap-4"
            onSubmit={async (event) => {
              event.preventDefault();
              const result = await reconnect.mutateAsync({ email, password });
              if (!result.mfaRequired) {
                setPassword("");
                setOpen(false);
              }
            }}
          >
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="settings-garmin-email">
                  Garmin email
                </FieldLabel>
                <Input
                  id="settings-garmin-email"
                  className="h-11"
                  type="email"
                  value={email}
                  onChange={(event) => setEmail(event.target.value)}
                  required
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="settings-garmin-password">
                  Garmin password
                </FieldLabel>
                <Input
                  id="settings-garmin-password"
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
                <FieldDescription>
                  A failed attempt never overwrites the last verified password.
                </FieldDescription>
              </Field>
            </FieldGroup>
            <DialogFooter showCloseButton>
              <Button type="submit" className="h-11" disabled={pending}>
                {reconnect.isPending ? <Spinner /> : null}
                {reconnect.isPending ? "Connecting…" : "Reconnect"}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}

function StravaDialog({ state }: { state: SettingsState }) {
  const [open, setOpen] = useState(false);
  const [clientId, setClientId] = useState(state.credentials.stravaClientId);
  const [clientSecret, setClientSecret] = useState("");
  const save = useSettingsAction(saveStravaCredentials);
  const disconnect = useSettingsAction(disconnectStrava);
  const canConnect =
    Boolean(clientId.trim()) &&
    (Boolean(clientSecret) || state.credentials.stravaClientSecretSaved);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="outline" className="h-10">
          {state.connections.strava.connected ? "Manage" : "Connect"}
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[calc(100vh-2rem)] overflow-y-auto sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Manage Strava</DialogTitle>
          <DialogDescription>
            Save credentials first, then authorize. Leave the secret blank to
            keep the saved one.
          </DialogDescription>
        </DialogHeader>
        <form
          className="grid gap-4"
          onSubmit={async (event) => {
            event.preventDefault();
            await save.mutateAsync({ clientId, clientSecret });
          }}
        >
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="settings-strava-id">
                Strava client ID
              </FieldLabel>
              <Input
                id="settings-strava-id"
                className="h-11"
                value={clientId}
                onChange={(event) => setClientId(event.target.value)}
                required
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="settings-strava-secret">
                Strava client secret
              </FieldLabel>
              <Input
                id="settings-strava-secret"
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
          <DialogFooter className="sm:flex-wrap">
            {state.connections.strava.connected ? (
              <Button
                type="button"
                variant="destructive"
                className="h-11"
                disabled={disconnect.isPending}
                onClick={async () => {
                  if (
                    !window.confirm(
                      "Disconnect Strava? Publishing pauses, but your activity list is kept.",
                    )
                  ) {
                    return;
                  }
                  await disconnect.mutateAsync();
                  setOpen(false);
                }}
              >
                Disconnect
              </Button>
            ) : (
              <DialogClose asChild>
                <Button variant="outline" className="h-11">
                  Cancel
                </Button>
              </DialogClose>
            )}
            <Button
              type="submit"
              variant="outline"
              className="h-11"
              disabled={save.isPending}
            >
              {save.isPending ? <Spinner /> : null}
              {save.isPending ? "Saving…" : "Save credentials"}
            </Button>
            <Button
              type="button"
              className="h-11"
              disabled={!canConnect || save.isPending}
              onClick={() => window.location.assign("/strava/connect")}
            >
              {state.connections.strava.connected ? "Reconnect" : "Connect"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function HevyDialog({ state }: { state: SettingsState }) {
  const [open, setOpen] = useState(false);
  const [apiKey, setApiKey] = useState("");
  const connect = useSettingsAction(saveHevyCredentials);
  const disconnect = useSettingsAction(disconnectHevy);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="outline" className="h-10">
          {state.hevy.connected ? "Manage" : "Connect"}
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[calc(100vh-2rem)] overflow-y-auto sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Manage Hevy</DialogTitle>
          <DialogDescription>
            Requires Hevy Pro. The key is validated before it is saved.
            Watch strategy and sync behavior live in Hevy integration below.
          </DialogDescription>
        </DialogHeader>
        {state.hevy.connected ? (
          <DialogFooter className="sm:flex-wrap">
            <Button
              type="button"
              variant="destructive"
              className="h-11"
              disabled={disconnect.isPending}
              onClick={async () => {
                if (!window.confirm("Disconnect Hevy and pause Hevy sync?")) {
                  return;
                }
                await disconnect.mutateAsync();
                setOpen(false);
              }}
            >
              {disconnect.isPending ? <Spinner /> : null}
              {disconnect.isPending ? "Disconnecting…" : "Disconnect"}
            </Button>
          </DialogFooter>
        ) : (
          <form
            className="grid gap-4"
            onSubmit={async (event) => {
              event.preventDefault();
              await connect.mutateAsync(apiKey);
              setApiKey("");
              setOpen(false);
            }}
          >
            <Field>
              <FieldLabel htmlFor="settings-hevy-api-key">
                Hevy API key
              </FieldLabel>
              <Input
                id="settings-hevy-api-key"
                className="h-11"
                type="password"
                value={apiKey}
                onChange={(event) => setApiKey(event.target.value)}
                required
              />
            </Field>
            <DialogFooter showCloseButton>
              <Button type="submit" className="h-11" disabled={connect.isPending}>
                {connect.isPending ? <Spinner /> : null}
                {connect.isPending ? "Validating…" : "Connect Hevy"}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
