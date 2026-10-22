import { useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useAuthAction } from "@/hooks/use-auth-action";
import { startOidcLink } from "@/lib/auth-api";

export function OidcLinkForm({
  displayName,
  onCancel,
}: {
  displayName: string;
  onCancel: () => void;
}) {
  const [currentPassword, setCurrentPassword] = useState("");
  const start = useAuthAction(startOidcLink);

  return (
    <form
      className="grid max-w-[520px] gap-4 rounded-lg border border-border/60 p-4"
      onSubmit={(event: FormEvent) => {
        event.preventDefault();
        start.mutate(currentPassword, {
          onSuccess: ({ url }) => window.location.assign(url),
        });
      }}
    >
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="oidc-link-password">Your ActivSync password</FieldLabel>
          <Input
            id="oidc-link-password"
            type="password"
            autoComplete="current-password"
            value={currentPassword}
            onChange={(e) => setCurrentPassword(e.target.value)}
            required
          />
          <FieldDescription>
            Required so a stolen session cannot link someone else’s account.
          </FieldDescription>
        </Field>
      </FieldGroup>
      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={start.isPending}>
          {start.isPending ? <Spinner /> : null}
          Continue to {displayName}
        </Button>
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
