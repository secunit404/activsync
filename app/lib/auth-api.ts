import { ApiError, requestJson } from "@/lib/api";

export type AuthMethod = "password" | "oidc" | "proxy" | "disabled";

export type AuthStatus = {
  authenticated: boolean;
  method: AuthMethod | null;
  user: string | null;
  ownerClaimed: boolean;
  authDisabled: boolean;
  login: {
    oidc: { label: string; autoRedirect: boolean; logoUrl: string | null } | null;
  };
};

export type AuthSettings = {
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
  redirectUri: string;
  linkedLabel: string | null;
  logoUrl: string | null;
};

export type OidcConfigInput = {
  displayName: string;
  issuer: string;
  clientId: string;
  clientSecret: string | null;
  scopes: string;
  autoRedirect: boolean;
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

export function startOidcLink(currentPassword: string) {
  return requestJson<{ url: string }>("/api/v1/auth/oidc/link", {
    method: "POST",
    body: JSON.stringify({ currentPassword }),
  });
}

export const OIDC_LOGO_MAX_BYTES = 1024 * 1024;
export const OIDC_LOGO_TYPES = "image/png,image/jpeg,image/webp";

export function uploadOidcLogo(file: File) {
  if (file.size > OIDC_LOGO_MAX_BYTES) {
    return Promise.reject(new ApiError("That image is too large. Use one under 1 MB.", 413));
  }
  return requestJson<AuthActionResult>("/api/v1/auth/oidc/logo", {
    method: "PUT",
    body: file,
    headers: { "Content-Type": file.type || "application/octet-stream" },
  });
}

export function removeOidcLogo() {
  return requestJson<AuthActionResult>("/api/v1/auth/oidc/logo", { method: "DELETE" });
}

export function oidcLoginUrl(next: string) {
  const params = new URLSearchParams({ intent: "login", next });
  return `/auth/oidc/login?${params.toString()}`;
}
