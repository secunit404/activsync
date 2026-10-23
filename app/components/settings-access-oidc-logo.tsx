import { useRef } from "react";

import { OidcLogo } from "@/components/oidc-logo";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { useAuthAction } from "@/hooks/use-auth-action";
import { OIDC_LOGO_TYPES, removeOidcLogo, uploadOidcLogo } from "@/lib/auth-api";

export function OidcLogoSettings({ logoUrl }: { logoUrl: string | null }) {
  const input = useRef<HTMLInputElement>(null);
  const upload = useAuthAction(uploadOidcLogo);
  const remove = useAuthAction(removeOidcLogo);
  const pending = upload.isPending || remove.isPending;

  return (
    <div className="grid gap-2">
      <span className="text-sm">Button logo</span>
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex h-9 min-w-9 items-center justify-center rounded-lg border border-input bg-muted px-2">
          <OidcLogo url={logoUrl} />
        </div>
        <Button
          type="button"
          variant="outline"
          disabled={pending}
          onClick={() => input.current?.click()}
        >
          {upload.isPending ? <Spinner /> : null}
          {logoUrl ? "Replace logo" : "Upload logo"}
        </Button>
        {logoUrl ? (
          <Button
            type="button"
            variant="ghost"
            disabled={pending}
            onClick={() => remove.mutate()}
          >
            Remove logo
          </Button>
        ) : null}
        <input
          ref={input}
          type="file"
          accept={OIDC_LOGO_TYPES}
          aria-label="Logo image"
          className="sr-only"
          tabIndex={-1}
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file) upload.mutate(file);
          }}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        PNG, JPEG or WebP, up to 1 MB. Shown on the sign-in button instead of the key.
      </p>
    </div>
  );
}
