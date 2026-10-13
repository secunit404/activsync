import { KeyRoundIcon } from "lucide-react";

import { oidcProviderIcon } from "@/lib/oidc-providers";

// Monochrome on purpose: several brand colours (Authelia's navy, Keycloak's
// grey) vanish on the dark muted button, and simple-icons' Google mark is a
// one-colour silhouette rather than the official four-colour "G" anyway.
export function OidcProviderIcon({
  provider,
  className,
}: {
  provider: string;
  className?: string;
}) {
  const icon = oidcProviderIcon(provider);
  if (!icon) return <KeyRoundIcon className={className} aria-hidden="true" />;
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      className={className}
      aria-hidden="true"
    >
      <path d={icon.path} />
    </svg>
  );
}
