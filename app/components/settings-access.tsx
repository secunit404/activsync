import { useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";

import { OidcSettings } from "@/components/settings-access-oidc";
import { SessionList } from "@/components/settings-access-sessions";
import { SettingsSection } from "@/components/settings-shell";
import { SignOutButton } from "@/components/sign-out-button";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { useAuthAction } from "@/hooks/use-auth-action";
import { useAuthStatus } from "@/hooks/use-auth-status";
import {
  changePassword,
  getAuthSettings,
  saveAuthSettings,
  type AuthSettings,
} from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";

/**
 * Settings → Access. Every control here saves on its own; none of it joins
 * the page's Discard/Save drafts footer, because a half-saved sign-in change
 * is worse than an immediate one.
 */
export function AccessSettings() {
  const status = useAuthStatus();
  const settings = useQuery({
    queryKey: queryKeys.authSettings,
    queryFn: ({ signal }) => getAuthSettings(signal),
  });

  return (
    <SettingsSection
      id="access"
      title="Access"
      description="How you sign in to ActivSync, and where you are signed in."
      contentClassName="grid gap-7"
    >
      {status.data?.method === "proxy" ? (
        <p className="text-sm">
          Signed in through your proxy as <strong>{status.data.user}</strong>.
        </p>
      ) : null}
      {settings.data && status.data ? (
        <>
          <PasswordChangeForm />
          <SignInMethods settings={settings.data} />
          <OidcSettings />
          <SessionList />
          <div>
            <SignOutButton method={status.data.method} variant="text" />
          </div>
        </>
      ) : (
        <div className="flex items-center gap-2 text-muted-foreground">
          <Spinner /> Loading sign-in settings…
        </div>
      )}
    </SettingsSection>
  );
}

function PasswordChangeForm() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const change = useAuthAction(changePassword);

  return (
    <form
      className="grid max-w-[440px] gap-4"
      onSubmit={(event: FormEvent) => {
        event.preventDefault();
        change.mutate(
          { current, new: next },
          {
            onSuccess: () => {
              setCurrent("");
              setNext("");
            },
          },
        );
      }}
    >
      <h3 className="text-sm font-semibold">Password</h3>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="access-current-password">Current password</FieldLabel>
          <Input
            id="access-current-password"
            type="password"
            autoComplete="current-password"
            value={current}
            onChange={(event) => setCurrent(event.target.value)}
            required
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="access-new-password">New password</FieldLabel>
          <Input
            id="access-new-password"
            type="password"
            autoComplete="new-password"
            minLength={15}
            value={next}
            onChange={(event) => setNext(event.target.value)}
            required
          />
        </Field>
      </FieldGroup>
      <Button type="submit" variant="outline" className="sm:w-fit" disabled={change.isPending}>
        {change.isPending ? <Spinner /> : null}
        Change password
      </Button>
    </form>
  );
}

function SignInMethods({ settings }: { settings: AuthSettings }) {
  const save = useAuthAction(saveAuthSettings);

  return (
    <div className="grid gap-4">
      <h3 className="text-sm font-semibold">Sign-in methods</h3>
      <label className="flex items-start justify-between gap-4 text-sm">
        <span className="grid gap-0.5">
          <span>Show the password form on the sign-in page</span>
          <span className="text-xs text-muted-foreground">
            When hidden, /login?local=1 still shows it.
          </span>
        </span>
        <Switch
          aria-label="Show the password form"
          checked={settings.passwordForm === "shown"}
          disabled={save.isPending}
          onCheckedChange={(checked) =>
            save.mutate({ passwordForm: checked === true ? "shown" : "hidden" })
          }
        />
      </label>
      <label className="flex items-start justify-between gap-4 text-sm">
        <span className="grid gap-0.5">
          <span>Sign in with Strava</span>
          <span className="text-xs text-muted-foreground">
            {settings.stravaLoginBlocker ??
              "Only the Strava athlete connected to ActivSync can sign in."}
          </span>
        </span>
        <Switch
          aria-label="Sign in with Strava"
          checked={settings.stravaLogin && !settings.stravaLoginBlocker}
          disabled={save.isPending || Boolean(settings.stravaLoginBlocker)}
          onCheckedChange={(checked) => save.mutate({ stravaLogin: checked === true })}
        />
      </label>
      {settings.proxy ? (
        <p className="text-xs text-muted-foreground">
          Proxy sign-in — configured via environment: {settings.proxy.header} from{" "}
          {settings.proxy.trusted.join(", ")}
        </p>
      ) : null}
    </div>
  );
}
