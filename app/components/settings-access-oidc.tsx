import { useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";

import { OidcLogoSettings } from "@/components/settings-access-oidc-logo";
import { Button } from "@/components/ui/button";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { useAuthAction } from "@/hooks/use-auth-action";
import {
  getOidcConfig,
  oidcLoginUrl,
  removeOidcConfig,
  saveOidcConfig,
  type OidcConfig,
} from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";

export function OidcSettings() {
  const config = useQuery({
    queryKey: queryKeys.oidcConfig,
    queryFn: ({ signal }) => getOidcConfig(signal),
  });
  const [editing, setEditing] = useState(false);

  if (!config.data) return null;
  const data = config.data;

  return (
    <div className="grid gap-3">
      <h3 className="text-sm font-semibold">Single sign-on (OIDC)</h3>
      {data.configured ? (
        <p className="text-sm text-muted-foreground">
          {data.displayName} at <span className="font-mono">{data.issuer}</span>
          {" — "}
          {data.linkedLabel ? `linked to ${data.linkedLabel}` : "not linked yet"}
        </p>
      ) : (
        <p className="text-sm text-muted-foreground">
          Sign in through your own identity provider, such as Authentik,
          Authelia, Pocket ID, Kanidm, or Google.
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" onClick={() => setEditing((open) => !open)}>
          {data.configured ? "Edit" : "Set up single sign-on"}
        </Button>
        {data.configured ? (
          <Button asChild variant="outline">
            <a href={oidcLoginUrl("/settings", "link")}>
              {data.linkedLabel ? "Re-link account" : "Link account"}
            </a>
          </Button>
        ) : null}
      </div>
      {data.configured ? <OidcLogoSettings logoUrl={data.logoUrl} /> : null}
      {editing ? <OidcForm config={data} onDone={() => setEditing(false)} /> : null}
    </div>
  );
}

function OidcForm({ config, onDone }: { config: OidcConfig; onDone: () => void }) {
  const [displayName, setDisplayName] = useState(config.displayName);
  const [issuer, setIssuer] = useState(config.issuer);
  const [clientId, setClientId] = useState(config.clientId);
  const [clientSecret, setClientSecret] = useState("");
  const [scopes, setScopes] = useState(config.scopes);
  const [autoRedirect, setAutoRedirect] = useState(config.autoRedirect);
  const [currentPassword, setCurrentPassword] = useState("");
  const save = useAuthAction(saveOidcConfig);
  const remove = useAuthAction(removeOidcConfig);
  const pending = save.isPending || remove.isPending;

  return (
    <form
      className="grid max-w-[520px] gap-4 rounded-lg border border-border/60 p-4"
      onSubmit={(event: FormEvent) => {
        event.preventDefault();
        save.mutate(
          {
            displayName,
            issuer,
            clientId,
            clientSecret: clientSecret === "" && config.hasSecret ? null : clientSecret,
            scopes,
            autoRedirect,
            currentPassword,
          },
          { onSuccess: onDone },
        );
      }}
    >
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="oidc-name">Provider name</FieldLabel>
          <Input id="oidc-name" value={displayName} onChange={(e) => setDisplayName(e.target.value)} required />
          <FieldDescription>Shown as “Sign in with {displayName || "…"}”.</FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor="oidc-issuer">Issuer URL</FieldLabel>
          <Input id="oidc-issuer" type="url" value={issuer} onChange={(e) => setIssuer(e.target.value)} required />
        </Field>
        <Field>
          <FieldLabel htmlFor="oidc-client-id">Client ID</FieldLabel>
          <Input id="oidc-client-id" value={clientId} onChange={(e) => setClientId(e.target.value)} required />
        </Field>
        <Field>
          <FieldLabel htmlFor="oidc-client-secret">Client secret</FieldLabel>
          <Input
            id="oidc-client-secret"
            type="password"
            autoComplete="off"
            value={clientSecret}
            placeholder={config.hasSecret ? "Saved — leave blank to keep it" : "Optional for public clients"}
            onChange={(e) => setClientSecret(e.target.value)}
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="oidc-scopes">Scopes</FieldLabel>
          <Input id="oidc-scopes" value={scopes} onChange={(e) => setScopes(e.target.value)} />
          <FieldDescription>
            Register this redirect URI with your provider:{" "}
            <span className="font-mono">{config.redirectUri}</span>
          </FieldDescription>
        </Field>
        <label className="flex items-center justify-between gap-4 text-sm">
          <span>Go straight to the provider from the sign-in page</span>
          <Switch
            aria-label="Redirect to the provider automatically"
            checked={autoRedirect}
            onCheckedChange={(checked) => setAutoRedirect(checked === true)}
          />
        </label>
        <Field>
          <FieldLabel htmlFor="oidc-current-password">Your ActivSync password</FieldLabel>
          <Input
            id="oidc-current-password"
            type="password"
            autoComplete="current-password"
            value={currentPassword}
            onChange={(e) => setCurrentPassword(e.target.value)}
            required
          />
          <FieldDescription>
            Required so a stolen session cannot point sign-in at another provider.
          </FieldDescription>
        </Field>
      </FieldGroup>
      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={pending}>
          {save.isPending ? <Spinner /> : null}
          Save
        </Button>
        {config.configured ? (
          <Button
            type="button"
            variant="destructive"
            disabled={pending || currentPassword === ""}
            onClick={() => remove.mutate(currentPassword, { onSuccess: onDone })}
          >
            Remove single sign-on
          </Button>
        ) : null}
        <Button type="button" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
