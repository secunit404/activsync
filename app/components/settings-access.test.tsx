import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { ApiError } from "@/lib/api";
import type { AuthSession, AuthSettings, AuthStatus, OidcConfig } from "@/lib/auth-api";
import { AccessSettings } from "./settings-access";

const api = vi.hoisted(() => ({
  getAuthStatus: vi.fn(),
  getAuthSettings: vi.fn(),
  getSessions: vi.fn(),
  getOidcConfig: vi.fn(),
  changePassword: vi.fn(),
  revokeSession: vi.fn(),
  revokeOtherSessions: vi.fn(),
  saveOidcConfig: vi.fn(),
  removeOidcConfig: vi.fn(),
  startOidcLink: vi.fn(),
  uploadOidcLogo: vi.fn(),
  removeOidcLogo: vi.fn(),
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
  login: { oidc: null },
};

const settings: AuthSettings = { proxy: null };

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
  logoUrl: null,
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
  api.changePassword.mockResolvedValue({ message: "Password changed." });
  api.revokeSession.mockResolvedValue(undefined);
  api.revokeOtherSessions.mockResolvedValue({ message: "Signed out 1 other session(s)." });
  api.saveOidcConfig.mockResolvedValue({ message: "Saved." });
});

afterEach(() => {
  vi.unstubAllGlobals();
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

test("shows the environment-configured proxy read-only", async () => {
  api.getAuthSettings.mockResolvedValue({
    proxy: { header: "Remote-User", trusted: ["172.18.0.0/16"], user: "andreas" },
  });
  renderAccess();

  expect(
    await screen.findByText(/Configured via environment: Remote-User from 172\.18\.0\.0\/16/),
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
  await userEvent.type(screen.getByLabelText("Provider name"), "Pocket ID");
  expect(screen.getByText("Shown as “Sign in with Pocket ID”.")).toBeVisible();
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

const configured: OidcConfig = {
  ...oidc,
  configured: true,
  displayName: "Pocket ID",
  issuer: "https://id.example.com",
  clientId: "activsync",
  hasSecret: true,
};

async function openOidcEditor() {
  api.getOidcConfig.mockResolvedValue(configured);
  renderAccess();
  await userEvent.click(await screen.findByRole("button", { name: "Edit" }));
  await userEvent.type(screen.getByLabelText("Your ActivSync password"), "my long passphrase");
}

test("editing other fields keeps the saved client secret, and the hint says so", async () => {
  await openOidcEditor();

  const secret = screen.getByLabelText("Client secret");
  expect(secret).toHaveAttribute("placeholder", "Saved — leave blank to keep it");
  await userEvent.clear(screen.getByLabelText("Provider name"));
  await userEvent.type(screen.getByLabelText("Provider name"), "Renamed");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));

  await vi.waitFor(() =>
    expect(api.saveOidcConfig).toHaveBeenCalledWith(
      expect.objectContaining({ displayName: "Renamed", clientSecret: null }),
      expect.anything(),
    ),
  );
});

test("a new client ID drops the saved-secret hint and sends no secret", async () => {
  await openOidcEditor();

  await userEvent.type(screen.getByLabelText("Client ID"), "-2");

  expect(screen.getByLabelText("Client secret")).toHaveAttribute(
    "placeholder",
    "Optional for public clients",
  );
  await userEvent.click(screen.getByRole("button", { name: "Save" }));

  await vi.waitFor(() =>
    expect(api.saveOidcConfig).toHaveBeenCalledWith(
      expect.objectContaining({ clientId: "activsync-2", clientSecret: "" }),
      expect.anything(),
    ),
  );
});

test("a new issuer drops the saved-secret hint and sends no secret", async () => {
  await openOidcEditor();

  const issuer = screen.getByLabelText("Issuer URL");
  await userEvent.clear(issuer);
  await userEvent.type(issuer, "https://sso.example.com");

  expect(screen.getByLabelText("Client secret")).toHaveAttribute(
    "placeholder",
    "Optional for public clients",
  );
  await userEvent.click(screen.getByRole("button", { name: "Save" }));

  await vi.waitFor(() =>
    expect(api.saveOidcConfig).toHaveBeenCalledWith(
      expect.objectContaining({ issuer: "https://sso.example.com", clientSecret: "" }),
      expect.anything(),
    ),
  );
});

test("restoring the saved issuer and client ID brings the saved-secret hint back", async () => {
  await openOidcEditor();
  const clientId = screen.getByLabelText("Client ID");

  await userEvent.type(clientId, "x");
  expect(screen.getByLabelText("Client secret")).toHaveAttribute(
    "placeholder",
    "Optional for public clients",
  );
  await userEvent.type(clientId, "{Backspace}");

  expect(screen.getByLabelText("Client secret")).toHaveAttribute(
    "placeholder",
    "Saved — leave blank to keep it",
  );
});

test("whitespace and a trailing slash are not counted as a changed issuer or client ID", async () => {
  await openOidcEditor();

  await userEvent.type(screen.getByLabelText("Issuer URL"), "/");
  await userEvent.type(screen.getByLabelText("Client ID"), " ");

  expect(screen.getByLabelText("Client secret")).toHaveAttribute(
    "placeholder",
    "Saved — leave blank to keep it",
  );
  await userEvent.click(screen.getByRole("button", { name: "Save" }));

  await vi.waitFor(() =>
    expect(api.saveOidcConfig).toHaveBeenCalledWith(
      expect.objectContaining({ clientSecret: null }),
      expect.anything(),
    ),
  );
});

test("a typed secret is always sent, even when the client ID changed", async () => {
  await openOidcEditor();

  await userEvent.type(screen.getByLabelText("Client ID"), "-2");
  await userEvent.type(screen.getByLabelText("Client secret"), "s3cret");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));

  await vi.waitFor(() =>
    expect(api.saveOidcConfig).toHaveBeenCalledWith(
      expect.objectContaining({ clientId: "activsync-2", clientSecret: "s3cret" }),
      expect.anything(),
    ),
  );
});

test("linking is a button that asks for the password, not a plain link", async () => {
  api.getOidcConfig.mockResolvedValue(configured);
  renderAccess();

  const trigger = await screen.findByRole("button", { name: "Link account" });
  expect(screen.queryByRole("link", { name: "Link account" })).toBeNull();
  expect(screen.getByText(/not linked yet/)).toBeVisible();
  expect(screen.queryByLabelText("Your ActivSync password")).toBeNull();

  await userEvent.click(trigger);

  const password = screen.getByLabelText("Your ActivSync password");
  expect(password).toHaveAttribute("type", "password");
  expect(password).toHaveAttribute("autocomplete", "current-password");
  expect(screen.getByRole("button", { name: "Continue to Pocket ID" })).toBeVisible();
  expect(api.startOidcLink).not.toHaveBeenCalled();
});

test("a linked provider offers to re-link", async () => {
  api.getOidcConfig.mockResolvedValue({ ...configured, linkedLabel: "andreas" });
  renderAccess();

  expect(await screen.findByRole("button", { name: "Re-link account" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Link account" })).toBeNull();
});

test("the password goes to the server and the browser follows the returned link", async () => {
  const assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, assign });
  api.getOidcConfig.mockResolvedValue(configured);
  api.startOidcLink.mockResolvedValue({ url: "/auth/oidc/login?intent=link&next=%2Fsettings" });
  renderAccess();

  await userEvent.click(await screen.findByRole("button", { name: "Link account" }));
  await userEvent.type(screen.getByLabelText("Your ActivSync password"), "my long passphrase");
  await userEvent.click(screen.getByRole("button", { name: "Continue to Pocket ID" }));

  await vi.waitFor(() =>
    expect(api.startOidcLink).toHaveBeenCalledWith("my long passphrase", expect.anything()),
  );
  await vi.waitFor(() =>
    expect(assign).toHaveBeenCalledWith("/auth/oidc/login?intent=link&next=%2Fsettings"),
  );
});

test("a wrong password stays on the page instead of leaving for the provider", async () => {
  const assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, assign });
  api.getOidcConfig.mockResolvedValue(configured);
  api.startOidcLink.mockRejectedValue(new ApiError("Wrong password.", 400));
  renderAccess();

  await userEvent.click(await screen.findByRole("button", { name: "Link account" }));
  await userEvent.type(screen.getByLabelText("Your ActivSync password"), "not it at all");
  await userEvent.click(screen.getByRole("button", { name: "Continue to Pocket ID" }));

  await vi.waitFor(() => expect(api.startOidcLink).toHaveBeenCalled());
  await vi.waitFor(() =>
    expect(screen.getByRole("button", { name: "Continue to Pocket ID" })).toBeEnabled(),
  );
  expect(assign).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Your ActivSync password")).toBeVisible();
});

test("cancelling the link step hides the password field", async () => {
  api.getOidcConfig.mockResolvedValue(configured);
  renderAccess();

  await userEvent.click(await screen.findByRole("button", { name: "Link account" }));
  const linkForm = screen.getByLabelText("Your ActivSync password").closest("form")!;
  await userEvent.click(within(linkForm).getByRole("button", { name: "Cancel" }));

  expect(screen.queryByLabelText("Your ActivSync password")).toBeNull();
});

test("single sign-on offers a logo upload once it is set up", async () => {
  api.getOidcConfig.mockResolvedValue(oidc);
  renderAccess();

  expect(await screen.findByRole("button", { name: "Set up single sign-on" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Upload logo" })).toBeNull();
});

test("choosing a logo uploads it straight away", async () => {
  api.getOidcConfig.mockResolvedValue(configured);
  api.uploadOidcLogo.mockResolvedValue({ message: "Logo saved." });
  renderAccess();
  const file = new File([new Uint8Array([137, 80, 78, 71])], "logo.png", { type: "image/png" });

  expect(await screen.findByRole("button", { name: "Upload logo" })).toBeVisible();
  const picker = screen.getByLabelText("Logo image");
  expect(picker).toHaveAttribute("accept", "image/png,image/jpeg,image/webp");
  await userEvent.upload(picker, file);

  await vi.waitFor(() =>
    expect(api.uploadOidcLogo).toHaveBeenCalledWith(file, expect.anything()),
  );
});

test("an uploaded logo is previewed and can be removed", async () => {
  const logoUrl = "/api/v1/auth/oidc/logo?v=0123456789ab";
  api.getOidcConfig.mockResolvedValue({ ...configured, logoUrl });
  api.removeOidcLogo.mockResolvedValue({ message: "Logo removed." });
  renderAccess();

  expect(await screen.findByRole("button", { name: "Replace logo" })).toBeVisible();
  expect(document.querySelector(`img[src="${logoUrl}"]`)).not.toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Remove logo" }));

  await vi.waitFor(() => expect(api.removeOidcLogo).toHaveBeenCalled());
});
