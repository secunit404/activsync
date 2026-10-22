import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { siAuthelia, siAuthentik, siGoogle, siKeycloak } from "simple-icons";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { ApiError } from "@/lib/api";
import type { AuthStatus } from "@/lib/auth-api";
import { LOGIN_ERRORS } from "@/lib/auth-errors";
import type { OidcProvider } from "@/lib/oidc-providers";
import Login, { safeNext } from "./login";

const { getAuthStatus, login } = vi.hoisted(() => ({
  getAuthStatus: vi.fn(),
  login: vi.fn(),
}));

vi.mock("@/lib/auth-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/auth-api")>();
  return { ...actual, getAuthStatus, login };
});

function status(overrides: Partial<AuthStatus> = {}): AuthStatus {
  return {
    authenticated: false,
    method: null,
    user: null,
    ownerClaimed: true,
    authDisabled: false,
    login: { oidc: null },
    ...overrides,
  };
}

function renderAt(path: string) {
  const router = createMemoryRouter(
    [
      { path: "/login", element: <Login /> },
      { path: "/setup", element: <p>setup page</p> },
      { path: "/hevy", element: <p>hevy page</p> },
      { path: "/", element: <p>home page</p> },
    ],
    { initialEntries: [path] },
  );
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return router;
}

beforeEach(() => {
  getAuthStatus.mockReset();
  login.mockReset();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

test("signs in with the password and returns to next", async () => {
  getAuthStatus.mockResolvedValue(status());
  login.mockResolvedValue(status({ authenticated: true, method: "password" }));
  const router = renderAt("/login?next=%2Fhevy");

  await userEvent.type(await screen.findByLabelText("Password"), "correct horse battery staple");
  await userEvent.click(screen.getByRole("button", { name: "Sign in" }));

  expect(await screen.findByText("hevy page")).toBeVisible();
  expect(login).toHaveBeenCalledWith("correct horse battery staple", expect.anything());
  expect(router.state.location.pathname).toBe("/hevy");
});

test("shows the server's message for a wrong password", async () => {
  getAuthStatus.mockResolvedValue(status());
  login.mockRejectedValue(new ApiError("Wrong password.", 401));
  renderAt("/login");

  await userEvent.type(await screen.findByLabelText("Password"), "nope");
  await userEvent.click(screen.getByRole("button", { name: "Sign in" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("Wrong password.");
});

test("the password field can be shown and hidden", async () => {
  getAuthStatus.mockResolvedValue(status());
  renderAt("/login");

  const input = await screen.findByLabelText("Password");
  expect(input).toHaveAttribute("type", "password");
  expect(input).toHaveAttribute("autocomplete", "current-password");

  await userEvent.type(input, "correct horse");
  await userEvent.click(screen.getByRole("button", { name: "Show password" }));

  expect(input).toHaveAttribute("type", "text");
  expect(input).toHaveValue("correct horse");
  expect(screen.getByRole("button", { name: "Hide password" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(login).not.toHaveBeenCalled();
});

test("maps error codes to fixed copy, never provider text", async () => {
  getAuthStatus.mockResolvedValue(status());
  renderAt("/login?error=oidc_mismatch&error_description=%3Cscript%3E");

  expect(await screen.findByText(LOGIN_ERRORS.oidc_mismatch)).toBeVisible();
  expect(screen.queryByText("<script>")).toBeNull();
});

test("offers single sign-on with next passed through", async () => {
  getAuthStatus.mockResolvedValue(
    status({ login: { oidc: { label: "Authentik", autoRedirect: false, provider: "authentik" } } }),
  );
  renderAt("/login?next=%2Fhevy");

  expect(await screen.findByRole("link", { name: "Sign in with Authentik" })).toHaveAttribute(
    "href",
    "/auth/oidc/login?intent=login&next=%2Fhevy",
  );
});

test.each([
  ["authentik", siAuthentik],
  ["authelia", siAuthelia],
  ["keycloak", siKeycloak],
  ["google", siGoogle],
] as const)("the %s sign-in button shows its logo", async (provider, icon) => {
  getAuthStatus.mockResolvedValue(
    status({ login: { oidc: { label: "My SSO", autoRedirect: false, provider } } }),
  );
  renderAt("/login");

  const link = await screen.findByRole("link", { name: "Sign in with My SSO" });
  const svg = link.querySelector("svg")!;
  expect(svg).toHaveAttribute("viewBox", "0 0 24 24");
  expect(svg).toHaveAttribute("fill", "currentColor");
  expect(svg).toHaveClass("size-5");
  expect(svg.querySelector("path")).toHaveAttribute("d", icon.path);
});

test.each(["pocket_id", "kanidm", "zitadel", "other", "unknown"])(
  "the %s sign-in button falls back to the key icon",
  async (provider) => {
    getAuthStatus.mockResolvedValue(
      status({
        login: {
          oidc: {
            label: "My SSO",
            autoRedirect: false,
            provider: provider as OidcProvider,
          },
        },
      }),
    );
    renderAt("/login");

    const link = await screen.findByRole("link", { name: "Sign in with My SSO" });
    expect(link.querySelector("svg")).toHaveClass("lucide-key-round", "size-5");
  },
);

test("the or divider shows only when a provider is offered", async () => {
  getAuthStatus.mockResolvedValue(status());
  renderAt("/login");

  expect(await screen.findByLabelText("Password")).toBeVisible();
  expect(screen.queryByText("or", { exact: true })).toBeNull();
});

test("the or divider separates the password form from single sign-on", async () => {
  getAuthStatus.mockResolvedValue(
    status({ login: { oidc: { label: "Authentik", autoRedirect: false, provider: "authentik" } } }),
  );
  renderAt("/login");

  expect(await screen.findByText("or", { exact: true })).toBeVisible();
  expect(screen.getByRole("link", { name: "Sign in with Authentik" })).toHaveClass("h-11", "w-full");
});

test("the sign-in page shows the brand, subtitle and a GitHub footer", async () => {
  getAuthStatus.mockResolvedValue(status());
  renderAt("/login");

  expect(await screen.findByLabelText("Password")).toBeVisible();
  expect(screen.getByLabelText("ActivSync")).toBeVisible();
  expect(screen.getByText("Sign in to manage your activities")).toBeVisible();
  expect(screen.getByRole("link", { name: "GitHub" })).toHaveAttribute(
    "href",
    "https://github.com/secunit404/activsync",
  );
});

test("?local=1 shows the password form and skips auto-redirect", async () => {
  const assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, assign });
  getAuthStatus.mockResolvedValue(
    status({
      login: { oidc: { label: "Authentik", autoRedirect: true, provider: "authentik" } },
    }),
  );
  renderAt("/login?local=1");

  expect(await screen.findByLabelText("Password")).toBeVisible();
  expect(assign).not.toHaveBeenCalled();
});

test("auto-redirect sends the browser straight to the provider", async () => {
  const assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, assign });
  getAuthStatus.mockResolvedValue(
    status({
      login: { oidc: { label: "Authentik", autoRedirect: true, provider: "authentik" } },
    }),
  );
  renderAt("/login?next=%2Fhevy");

  await vi.waitFor(() =>
    expect(assign).toHaveBeenCalledWith("/auth/oidc/login?intent=login&next=%2Fhevy"),
  );
});

test("an unclaimed install goes to the claim step", async () => {
  getAuthStatus.mockResolvedValue(status({ ownerClaimed: false }));
  renderAt("/login");

  expect(await screen.findByText("setup page")).toBeVisible();
});

test("an already signed-in browser skips the form", async () => {
  getAuthStatus.mockResolvedValue(status({ authenticated: true, method: "password" }));
  renderAt("/login?next=%2Fhevy");

  expect(await screen.findByText("hevy page")).toBeVisible();
});

test("next never leaves the site", () => {
  expect(safeNext("//evil.test")).toBe("/");
  expect(safeNext("https://evil.test")).toBe("/");
  expect(safeNext("/\\evil.test")).toBe("/");
  expect(safeNext("/\t/evil.test")).toBe("/");
  expect(safeNext("/\n/evil.test")).toBe("/");
  expect(safeNext("/\r/evil.test")).toBe("/");
  expect(safeNext("/.//evil.test")).toBe("/");
  expect(safeNext("/a/..//evil.test")).toBe("/");
  expect(safeNext("/./\\evil.test")).toBe("/");
  expect(safeNext("/hevy?x=1")).toBe("/hevy?x=1");
  expect(safeNext("/hevy#top")).toBe("/hevy#top");
  expect(safeNext(null)).toBe("/");
});
