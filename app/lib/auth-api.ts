import { requestJson } from "@/lib/api";

export type AuthMethod = "password" | "oidc" | "strava" | "proxy" | "disabled";
export type PasswordForm = "shown" | "hidden";

export type AuthStatus = {
  authenticated: boolean;
  method: AuthMethod | null;
  user: string | null;
  ownerClaimed: boolean;
  authDisabled: boolean;
  login: {
    passwordForm: PasswordForm;
    oidc: { label: string; autoRedirect: boolean } | null;
    strava: boolean;
  };
};

export type AuthSettings = {
  passwordForm: PasswordForm;
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

export function saveAuthSettings(payload: { passwordForm?: PasswordForm }) {
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

export function oidcLoginUrl(next: string, intent: "login" | "link" = "login") {
  const params = new URLSearchParams({ intent, next });
  return `/auth/oidc/login?${params.toString()}`;
}

export function stravaLoginUrl(next: string) {
  return `/auth/strava/login?${new URLSearchParams({ next }).toString()}`;
}
