import {
  siAuthelia,
  siAuthentik,
  siGoogle,
  siKeycloak,
  type SimpleIcon,
} from "simple-icons";

export type OidcProvider =
  | "authentik"
  | "authelia"
  | "keycloak"
  | "pocket_id"
  | "kanidm"
  | "zitadel"
  | "google"
  | "other";

type ProviderInfo = { label: string; icon: SimpleIcon | null };

// simple-icons has no Pocket ID, Kanidm, or Zitadel logo, so those show the
// generic key like "other" does.
export const OIDC_PROVIDERS: Record<OidcProvider, ProviderInfo> = {
  authentik: { label: "Authentik", icon: siAuthentik },
  authelia: { label: "Authelia", icon: siAuthelia },
  keycloak: { label: "Keycloak", icon: siKeycloak },
  pocket_id: { label: "Pocket ID", icon: null },
  kanidm: { label: "Kanidm", icon: null },
  zitadel: { label: "Zitadel", icon: null },
  google: { label: "Google", icon: siGoogle },
  other: { label: "Other", icon: null },
};

export const OIDC_PROVIDER_IDS = Object.keys(OIDC_PROVIDERS) as OidcProvider[];

export function oidcProviderIcon(provider: string): SimpleIcon | null {
  return Object.hasOwn(OIDC_PROVIDERS, provider)
    ? OIDC_PROVIDERS[provider as OidcProvider].icon
    : null;
}

export function defaultOidcName(provider: OidcProvider): string {
  return provider === "other" ? "" : OIDC_PROVIDERS[provider].label;
}
