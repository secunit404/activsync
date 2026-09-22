import { useState, type ReactNode } from "react";

import { ConnectionStatus, SettingsSection } from "@/components/settings-shell";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import {
  Field,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Hint } from "@/components/ui/hint";
import { Input } from "@/components/ui/input";
import { ResponsiveOverlay } from "@/components/ui/responsive-overlay";
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
            size="xl"
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
            size="xl"
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

/**
 * Footer row shared by the three connection sheets. `ResponsiveOverlay` keeps
 * its footer outside the scrolling body, so the buttons can't live inside the
 * `<form>` element they submit — they reach it by `form={id}` instead, which
 * is what the attribute is for.
 */
function ConnectionFooter({ children }: { children: ReactNode }) {
  return (
    <div className="flex flex-col-reverse gap-2.5 md:flex-row md:justify-end">
      {children}
    </div>
  );
}

const GARMIN_FORM_ID = "settings-garmin-form";
const STRAVA_FORM_ID = "settings-strava-form";
const HEVY_FORM_ID = "settings-hevy-form";

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
    <>
      <Button variant="outline" size="lg" onClick={() => setOpen(true)}>
        {state.connections.garmin.connected ? "Manage" : "Reconnect"}
      </Button>
      <ResponsiveOverlay
        open={open}
        onOpenChange={setOpen}
        title="Manage Garmin"
        description={
          state.setup.mfaRequired
            ? "Enter the verification code Garmin sent."
            : "Leave the password blank to keep the saved one."
        }
        mobile="sheet"
        footer={
          <ConnectionFooter>
            {state.setup.mfaRequired ? (
              <>
                <Button
                  type="button"
                  variant="outline"
                  size="xl"
                  disabled={pending}
                  onClick={async () => {
                    await cancelMfa.mutateAsync();
                    setOpen(false);
                  }}
                >
                  Cancel verification
                </Button>
                <Button
                  type="submit"
                  form={GARMIN_FORM_ID}
                  size="xl"
                  disabled={pending}
                >
                  {mfa.isPending ? <Spinner /> : null}
                  {mfa.isPending ? "Verifying…" : "Verify code"}
                </Button>
              </>
            ) : (
              <Button
                type="submit"
                form={GARMIN_FORM_ID}
                size="xl"
                disabled={pending}
              >
                {reconnect.isPending ? <Spinner /> : null}
                {reconnect.isPending ? "Connecting…" : "Reconnect"}
              </Button>
            )}
          </ConnectionFooter>
        }
      >
        {state.setup.mfaRequired ? (
          <form
            id={GARMIN_FORM_ID}
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
              />
            </Field>
          </form>
        ) : (
          <form
            id={GARMIN_FORM_ID}
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
                <div className="flex items-center gap-0.5">
                  <FieldLabel htmlFor="settings-garmin-password">
                    Garmin password
                  </FieldLabel>
                  <Hint label="Garmin password">
                    A failed attempt never overwrites the last verified
                    password.
                  </Hint>
                </div>
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
              </Field>
            </FieldGroup>
          </form>
        )}
      </ResponsiveOverlay>
    </>
  );
}

function StravaDialog({ state }: { state: SettingsState }) {
  const [open, setOpen] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const [clientId, setClientId] = useState(state.credentials.stravaClientId);
  const [clientSecret, setClientSecret] = useState("");
  const save = useSettingsAction(saveStravaCredentials);
  const disconnect = useSettingsAction(disconnectStrava);
  const canConnect =
    Boolean(clientId.trim()) &&
    (Boolean(clientSecret) || state.credentials.stravaClientSecretSaved);
  return (
    <>
      <Button variant="outline" size="lg" onClick={() => setOpen(true)}>
        {state.connections.strava.connected ? "Manage" : "Connect"}
      </Button>
      <ResponsiveOverlay
        open={open}
        onOpenChange={setOpen}
        title="Manage Strava"
        description="Saving takes you to Strava to authorize. Leave the secret blank to keep the saved one."
        mobile="sheet"
        footer={
          <ConnectionFooter>
            {state.connections.strava.connected ? (
              <Button
                type="button"
                variant="destructive"
                size="xl"
                disabled={disconnect.isPending}
                onClick={() => setConfirmDisconnect(true)}
              >
                Disconnect
              </Button>
            ) : (
              <Button
                type="button"
                variant="outline"
                size="xl"
                onClick={() => setOpen(false)}
              >
                Cancel
              </Button>
            )}
            <Button
              type="submit"
              form={STRAVA_FORM_ID}
              size="xl"
              disabled={!canConnect || save.isPending}
            >
              {save.isPending ? <Spinner /> : null}
              {save.isPending
                ? "Saving…"
                : state.connections.strava.connected
                  ? "Save & reconnect"
                  : "Save & connect"}
            </Button>
          </ConnectionFooter>
        }
      >
        <form
          id={STRAVA_FORM_ID}
          className="grid gap-4"
          onSubmit={async (event) => {
            event.preventDefault();
            // Save first, then hand off to Strava's OAuth screen. These were
            // two separate buttons; they are one intent, and splitting them
            // let a user authorize against credentials they hadn't saved.
            await save.mutateAsync({ clientId, clientSecret });
            window.location.assign("/strava/connect");
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
        </form>
      </ResponsiveOverlay>
      <ConfirmDialog
        open={confirmDisconnect}
        onOpenChange={setConfirmDisconnect}
        title="Disconnect Strava?"
        description="Publishing pauses, but your activity list is kept."
        confirmLabel="Disconnect"
        pendingLabel="Disconnecting…"
        pending={disconnect.isPending}
        onConfirm={async () => {
          await disconnect.mutateAsync();
          setConfirmDisconnect(false);
          setOpen(false);
        }}
      />
    </>
  );
}

function HevyDialog({ state }: { state: SettingsState }) {
  const [open, setOpen] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const [apiKey, setApiKey] = useState("");
  const connect = useSettingsAction(saveHevyCredentials);
  const disconnect = useSettingsAction(disconnectHevy);

  return (
    <>
      <Button variant="outline" size="lg" onClick={() => setOpen(true)}>
        {state.hevy.connected ? "Manage" : "Connect"}
      </Button>
      <ResponsiveOverlay
        open={open}
        onOpenChange={setOpen}
        title="Manage Hevy"
        description="Requires Hevy Pro. The key is validated before it is saved. Watch strategy and sync behavior live in Hevy integration below."
        mobile="sheet"
        footer={
          <ConnectionFooter>
            {state.hevy.connected ? (
              <>
                <Button
                  type="button"
                  variant="destructive"
                  size="xl"
                  disabled={disconnect.isPending}
                  onClick={() => setConfirmDisconnect(true)}
                >
                  {disconnect.isPending ? <Spinner /> : null}
                  {disconnect.isPending ? "Disconnecting…" : "Disconnect"}
                </Button>
                <Button
                  type="submit"
                  form={HEVY_FORM_ID}
                  size="xl"
                  disabled={!apiKey.trim() || connect.isPending}
                >
                  {connect.isPending ? <Spinner /> : null}
                  {connect.isPending ? "Validating…" : "Save new key"}
                </Button>
              </>
            ) : (
              <Button
                type="submit"
                form={HEVY_FORM_ID}
                size="xl"
                disabled={connect.isPending}
              >
                {connect.isPending ? <Spinner /> : null}
                {connect.isPending ? "Validating…" : "Connect Hevy"}
              </Button>
            )}
          </ConnectionFooter>
        }
      >
        {state.hevy.connected ? (
          // Connected, this dialog used to be a Disconnect button and
          // nothing else — there was no way to rotate a key without
          // disconnecting first. Replacing it reuses the same validated
          // save path the connect form uses.
          <form
            id={HEVY_FORM_ID}
            className="grid gap-4"
            onSubmit={async (event) => {
              event.preventDefault();
              await connect.mutateAsync(apiKey);
              setApiKey("");
              setOpen(false);
            }}
          >
            <p className="rounded-lg border border-border/70 bg-background p-3 font-mono text-xs text-muted-foreground">
              API key saved · {state.hevy.status}
            </p>
            <Field>
              <div className="flex items-center gap-0.5">
                <FieldLabel htmlFor="settings-hevy-replace-key">
                  Replace API key
                </FieldLabel>
                <Hint label="Replace API key">
                  The new key is validated against Hevy before it replaces the
                  saved one.
                </Hint>
              </div>
              <Input
                id="settings-hevy-replace-key"
                className="h-11"
                type="password"
                autoComplete="new-password"
                value={apiKey}
                onChange={(event) => setApiKey(event.target.value)}
                placeholder="Leave blank to keep the saved key"
              />
            </Field>
          </form>
        ) : (
          <form
            id={HEVY_FORM_ID}
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
          </form>
        )}
      </ResponsiveOverlay>
      <ConfirmDialog
        open={confirmDisconnect}
        onOpenChange={setConfirmDisconnect}
        title="Disconnect Hevy?"
        description="This also pauses Hevy sync."
        confirmLabel="Disconnect"
        pendingLabel="Disconnecting…"
        pending={disconnect.isPending}
        onConfirm={async () => {
          await disconnect.mutateAsync();
          setConfirmDisconnect(false);
          setOpen(false);
        }}
      />
    </>
  );
}
