import { requestJson, type AuthorizeUrl } from "@/lib/api";
import type { OidcProvider } from "@/lib/oidc-providers";

export type AuthMethod = "password" | "oidc" | "strava" | "proxy" | "disabled";

export type AuthStatus = {
  authenticated: boolean;
  method: AuthMethod | null;
  user: string | null;
  ownerClaimed: boolean;
  authDisabled: boolean;
  login: {
    oidc: { label: string; autoRedirect: boolean; provider: OidcProvider } | null;
    strava: boolean;
  };
};

export type AuthSettings = {
  stravaLogin: boolean;
  stravaLoginBlocker: string | null;
  proxy: { header: string; trusted: string[]; user: string } | null;
};

export type AuthSession = {
  publicId: string;
  method: string;
  createdAt: string;
  lastSeenAt: string;
  ip: string;
  userAgent: string;
  current: boolean;
};

export type OidcConfig = {
  configured: boolean;
  displayName: string;
  issuer: string;
  clientId: string;
  hasSecret: boolean;
  scopes: string;
  autoRedirect: boolean;
  provider: OidcProvider;
  redirectUri: string;
  linkedLabel: string | null;
};

export type OidcConfigInput = {
  displayName: string;
  issuer: string;
  clientId: string;
  clientSecret: string | null;
  scopes: string;
  autoRedirect: boolean;
  provider: OidcProvider;
  currentPassword: string;
};

export type AuthActionResult = { message: string };

export function getAuthStatus(signal?: AbortSignal): Promise<AuthStatus> {
  return requestJson<AuthStatus>("/api/v1/auth/status", { signal });
}

export function claimOwner(payload: { setupCode: string; password: string }) {
  return requestJson<AuthStatus>("/api/v1/auth/claim", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function login(password: string) {
  return requestJson<AuthStatus>("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ password }),
  });
}

export function logout() {
  return requestJson<void>("/api/v1/auth/logout", { method: "POST" });
}

export function getAuthSettings(signal?: AbortSignal): Promise<AuthSettings> {
  return requestJson<AuthSettings>("/api/v1/auth/settings", { signal });
}

export function saveAuthSettings(payload: { stravaLogin?: boolean }) {
  return requestJson<AuthActionResult>("/api/v1/auth/settings", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function changePassword(payload: { current: string; new: string }) {
  return requestJson<AuthActionResult>("/api/v1/auth/password", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function getSessions(signal?: AbortSignal): Promise<AuthSession[]> {
  return requestJson<AuthSession[]>("/api/v1/auth/sessions", { signal });
}

export function revokeSession(publicId: string) {
  return requestJson<void>(
    `/api/v1/auth/sessions/${encodeURIComponent(publicId)}`,
    { method: "DELETE" },
  );
}

export function revokeOtherSessions() {
  return requestJson<AuthActionResult>("/api/v1/auth/sessions", {
    method: "DELETE",
  });
}

export function getOidcConfig(signal?: AbortSignal): Promise<OidcConfig> {
  return requestJson<OidcConfig>("/api/v1/auth/oidc", { signal });
}

export function saveOidcConfig(payload: OidcConfigInput) {
  return requestJson<AuthActionResult>("/api/v1/auth/oidc", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function removeOidcConfig(currentPassword: string) {
  return requestJson<AuthActionResult>("/api/v1/auth/oidc", {
    method: "DELETE",
    body: JSON.stringify({ currentPassword }),
  });
}

export function oidcLoginUrl(next: string, intent: "login" | "link" = "login") {
  const params = new URLSearchParams({ intent, next });
  return `/auth/oidc/login?${params.toString()}`;
}

export function startStravaLogin(next: string, signal?: AbortSignal): Promise<AuthorizeUrl> {
  return requestJson<AuthorizeUrl>("/api/v1/auth/strava/authorize", {
    method: "POST",
    body: JSON.stringify({ next }),
    signal,
  });
}
