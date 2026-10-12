import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import type { AuthSession, AuthSettings, AuthStatus } from "@/lib/auth-api";
import { AccessSettings } from "./settings-access";

const api = vi.hoisted(() => ({
  getAuthStatus: vi.fn(),
  getAuthSettings: vi.fn(),
  getSessions: vi.fn(),
  saveAuthSettings: vi.fn(),
  changePassword: vi.fn(),
  revokeSession: vi.fn(),
  revokeOtherSessions: vi.fn(),
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
  login: { passwordForm: "shown", oidc: null, strava: false },
};

const settings: AuthSettings = { passwordForm: "shown" };

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
  api.saveAuthSettings.mockResolvedValue({ message: "Sign-in settings saved." });
  api.changePassword.mockResolvedValue({ message: "Password changed." });
  api.revokeSession.mockResolvedValue(undefined);
  api.revokeOtherSessions.mockResolvedValue({ message: "Signed out 1 other session(s)." });
});

test("changes the password with the current one", async () => {
  renderAccess();

  await userEvent.type(await screen.findByLabelText("Current password"), "old long passphrase");
  await userEvent.type(screen.getByLabelText("New password"), "a new long passphrase");
  await userEvent.click(screen.getByRole("button", { name: "Change password" }));

  await vi.waitFor(() =>
    expect(api.changePassword).toHaveBeenCalledWith(
      { current: "old long passphrase", new: "a new long passphrase" },
      expect.anything(),
    ),
  );
});

test("hiding the password form saves immediately", async () => {
  renderAccess();

  await userEvent.click(await screen.findByRole("switch", { name: "Show the password form" }));

  await vi.waitFor(() =>
    expect(api.saveAuthSettings).toHaveBeenCalledWith({ passwordForm: "hidden" }, expect.anything()),
  );
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
