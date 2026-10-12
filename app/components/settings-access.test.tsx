import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import type { AuthSession, AuthSettings, AuthStatus, OidcConfig } from "@/lib/auth-api";
import { AccessSettings } from "./settings-access";

const api = vi.hoisted(() => ({
  getAuthStatus: vi.fn(),
  getAuthSettings: vi.fn(),
  getSessions: vi.fn(),
  getOidcConfig: vi.fn(),
  saveAuthSettings: vi.fn(),
  changePassword: vi.fn(),
  revokeSession: vi.fn(),
  revokeOtherSessions: vi.fn(),
  saveOidcConfig: vi.fn(),
  removeOidcConfig: vi.fn(),
}));

vi.mock("@/lib/auth-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/auth-api")>();
  return { ...actual, ...api };
});

const signedIn: AuthStatus = {
  authenticated: true,
  method: "password",
  user: null,
  ownerClaimed: true,
  authDisabled: false,
  login: { oidc: null, strava: false },
};

const settings: AuthSettings = {
  stravaLogin: false,
  stravaLoginBlocker: null,
  proxy: null,
};

const sessions: AuthSession[] = [
  {
    publicId: "current-id",
    method: "password",
    createdAt: "2026-10-12T08:00:00+00:00",
    lastSeenAt: "2026-10-12T09:00:00+00:00",
    ip: "192.0.2.10",
    userAgent: "Firefox",
    current: true,
  },
  {
    publicId: "phone-id",
    method: "oidc",
    createdAt: "2026-10-10T08:00:00+00:00",
    lastSeenAt: "2026-10-11T08:00:00+00:00",
    ip: "198.51.100.4",
    userAgent: "Safari",
    current: false,
  },
];

const oidc: OidcConfig = {
  configured: false,
  displayName: "",
  issuer: "",
  clientId: "",
  hasSecret: false,
  scopes: "openid profile email",
  autoRedirect: false,
  redirectUri: "https://activsync.example.com/auth/oidc/callback",
  linkedLabel: null,
};

function renderAccess() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <AccessSettings />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  for (const mock of Object.values(api)) mock.mockReset();
  api.getAuthStatus.mockResolvedValue(signedIn);
  api.getAuthSettings.mockResolvedValue(settings);
  api.getSessions.mockResolvedValue(sessions);
  api.getOidcConfig.mockResolvedValue(oidc);
  api.saveAuthSettings.mockResolvedValue({ message: "Sign-in settings saved." });
  api.changePassword.mockResolvedValue({ message: "Password changed." });
  api.revokeSession.mockResolvedValue(undefined);
  api.revokeOtherSessions.mockResolvedValue({ message: "Signed out 1 other session(s)." });
  api.saveOidcConfig.mockResolvedValue({ message: "Saved." });
});

test("changes the password with the current one", async () => {
  renderAccess();

  await userEvent.type(await screen.findByLabelText("Current password"), "old long passphrase");
  await userEvent.type(screen.getByLabelText("New password"), "a new long passphrase");
  await userEvent.type(screen.getByLabelText("Repeat new password"), "a new long passphrase");
  await userEvent.click(screen.getByRole("button", { name: "Change password" }));

  await vi.waitFor(() =>
    expect(api.changePassword).toHaveBeenCalledWith(
      { current: "old long passphrase", new: "a new long passphrase" },
      expect.anything(),
    ),
  );
});

test("a mismatched repeat blocks the password change", async () => {
  renderAccess();

  await userEvent.type(await screen.findByLabelText("Current password"), "old long passphrase");
  await userEvent.type(screen.getByLabelText("New password"), "a new long passphrase");
  await userEvent.type(screen.getByLabelText("Repeat new password"), "a new long passphrasx");

  expect(screen.getByText("The new passwords don’t match.")).toBeVisible();
  const submit = screen.getByRole("button", { name: "Change password" });
  expect(submit).toBeDisabled();
  await userEvent.click(submit);
  expect(api.changePassword).not.toHaveBeenCalled();
});

test("each password field has its own show/hide toggle", async () => {
  renderAccess();

  const current = await screen.findByLabelText("Current password");
  const next = screen.getByLabelText("New password");
  const repeat = screen.getByLabelText("Repeat new password");
  expect(current).toHaveAttribute("autocomplete", "current-password");
  expect(next).toHaveAttribute("autocomplete", "new-password");
  expect(repeat).toHaveAttribute("autocomplete", "new-password");

  const form = current.closest("form")!;
  const toggles = within(form).getAllByRole("button", { name: "Show password" });
  expect(toggles).toHaveLength(3);
  await userEvent.click(toggles[1]);

  expect(current).toHaveAttribute("type", "password");
  expect(next).toHaveAttribute("type", "text");
  expect(repeat).toHaveAttribute("type", "password");
  expect(within(form).getByRole("button", { name: "Hide password" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("the Strava toggle waits for its precondition", async () => {
  api.getAuthSettings.mockResolvedValue({
    ...settings,
    stravaLoginBlocker: "Waiting for the next Strava sync.",
  });
  renderAccess();

  expect(await screen.findByText("Waiting for the next Strava sync.")).toBeVisible();
  expect(screen.getByRole("switch", { name: "Sign in with Strava" })).toBeDisabled();
});

test("turning on Sign in with Strava saves immediately", async () => {
  renderAccess();

  await userEvent.click(await screen.findByRole("switch", { name: "Sign in with Strava" }));

  await vi.waitFor(() =>
    expect(api.saveAuthSettings).toHaveBeenCalledWith({ stravaLogin: true }, expect.anything()),
  );
});

test("shows the environment-configured proxy read-only", async () => {
  api.getAuthSettings.mockResolvedValue({
    ...settings,
    proxy: { header: "Remote-User", trusted: ["172.18.0.0/16"], user: "andreas" },
  });
  renderAccess();

  expect(
    await screen.findByText(/configured via environment: Remote-User from 172\.18\.0\.0\/16/),
  ).toBeVisible();
});

test("a proxy sign-in says so and offers no sign-out", async () => {
  api.getAuthStatus.mockResolvedValue({ ...signedIn, method: "proxy", user: "andreas" });
  renderAccess();

  expect(await screen.findByText(/Signed in through your proxy as/)).toHaveTextContent("andreas");
  expect(screen.queryByRole("button", { name: "Sign out" })).toBeNull();
});

test("lists sessions and revokes another one", async () => {
  renderAccess();

  const list = await screen.findByRole("list", { name: "Signed-in sessions" });
  expect(await within(list).findByText("This browser")).toBeVisible();
  await userEvent.click(within(list).getByRole("button", { name: /Single sign-on session from 198\.51\.100\.4/ }));

  await vi.waitFor(() => expect(api.revokeSession).toHaveBeenCalledWith("phone-id", expect.anything()));
});

test("signs out everywhere else", async () => {
  renderAccess();

  await userEvent.click(await screen.findByRole("button", { name: "Sign out everywhere else" }));

  await vi.waitFor(() => expect(api.revokeOtherSessions).toHaveBeenCalled());
});

test("offers a sign-out button for password sessions", async () => {
  renderAccess();

  expect(await screen.findByRole("button", { name: "Sign out" })).toBeVisible();
});

test("configuring single sign-on sends the current password and shows the redirect URI", async () => {
  renderAccess();

  await userEvent.click(await screen.findByRole("button", { name: "Set up single sign-on" }));
  expect(screen.getByText("https://activsync.example.com/auth/oidc/callback")).toBeVisible();
  await userEvent.type(screen.getByLabelText("Button label"), "Pocket ID");
  await userEvent.type(screen.getByLabelText("Issuer URL"), "https://id.example.com");
  await userEvent.type(screen.getByLabelText("Client ID"), "activsync");
  await userEvent.type(screen.getByLabelText("Your ActivSync password"), "my long passphrase");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));

  await vi.waitFor(() =>
    expect(api.saveOidcConfig).toHaveBeenCalledWith(
      {
        displayName: "Pocket ID",
        issuer: "https://id.example.com",
        clientId: "activsync",
        clientSecret: "",
        scopes: "openid profile email",
        autoRedirect: false,
        currentPassword: "my long passphrase",
      },
      expect.anything(),
    ),
  );
});

test("a configured provider offers to link the account", async () => {
  api.getOidcConfig.mockResolvedValue({
    ...oidc,
    configured: true,
    displayName: "Pocket ID",
    issuer: "https://id.example.com",
    clientId: "activsync",
    hasSecret: true,
  });
  renderAccess();

  expect(await screen.findByRole("link", { name: "Link account" })).toHaveAttribute(
    "href",
    "/auth/oidc/login?intent=link&next=%2Fsettings",
  );
  expect(screen.getByText(/not linked yet/)).toBeVisible();
});
