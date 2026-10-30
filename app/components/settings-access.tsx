import { useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";

import { OidcSettings } from "@/components/settings-access-oidc";
import { SessionList } from "@/components/settings-access-sessions";
import { SettingsSection } from "@/components/settings-shell";
import { SignOutButton } from "@/components/sign-out-button";
import { Button } from "@/components/ui/button";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { PasswordInput } from "@/components/ui/password-input";
import { Spinner } from "@/components/ui/spinner";
import { useAuthAction } from "@/hooks/use-auth-action";
import { useAuthStatus } from "@/hooks/use-auth-status";
import { changePassword, getAuthSettings, type AuthSettings } from "@/lib/auth-api";
import { passwordsMismatch } from "@/lib/password";
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
      contentClassName="grid grid-cols-[minmax(0,1fr)] gap-7"
    >
      {status.data?.method === "proxy" ? (
        <p className="text-sm">
          Signed in through your proxy as <strong>{status.data.user}</strong>.
        </p>
      ) : null}
      {settings.data && status.data ? (
        <>
          <PasswordChangeForm />
          {settings.data.proxy ? <ProxySignIn proxy={settings.data.proxy} /> : null}
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
  const [repeat, setRepeat] = useState("");
  const change = useAuthAction(changePassword);
  const mismatch = passwordsMismatch(next, repeat);

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
              setRepeat("");
            },
          },
        );
      }}
    >
      <h3 className="text-sm font-semibold">Password</h3>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="access-current-password">Current password</FieldLabel>
          <PasswordInput
            id="access-current-password"
            autoComplete="current-password"
            value={current}
            onChange={(event) => setCurrent(event.target.value)}
            required
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="access-new-password">New password</FieldLabel>
          <PasswordInput
            id="access-new-password"
            autoComplete="new-password"
            minLength={15}
            value={next}
            onChange={(event) => setNext(event.target.value)}
            required
          />
        </Field>
        <Field data-invalid={mismatch || undefined}>
          <FieldLabel htmlFor="access-new-password-repeat">Repeat new password</FieldLabel>
          <PasswordInput
            id="access-new-password-repeat"
            autoComplete="new-password"
            value={repeat}
            onChange={(event) => setRepeat(event.target.value)}
            required
          />
          {mismatch ? (
            <FieldDescription>The new passwords don’t match.</FieldDescription>
          ) : null}
        </Field>
      </FieldGroup>
      <Button
        type="submit"
        variant="outline"
        className="sm:w-fit"
        disabled={change.isPending || mismatch}
      >
        {change.isPending ? <Spinner /> : null}
        Change password
      </Button>
    </form>
  );
}

function ProxySignIn({ proxy }: { proxy: NonNullable<AuthSettings["proxy"]> }) {
  return (
    <div className="grid gap-1">
      <h3 className="text-sm font-semibold">Proxy sign-in</h3>
      <p className="text-xs text-muted-foreground">
        Configured via environment: {proxy.header} from {proxy.trusted.join(", ")}
      </p>
    </div>
  );
}
