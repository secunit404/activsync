import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { ApiError } from "@/lib/api";
import type { AuthStatus } from "@/lib/auth-api";
import { LOGIN_ERRORS } from "@/lib/auth-errors";
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
    login: { oidc: null, strava: false },
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

test("maps error codes to fixed copy, never provider text", async () => {
  getAuthStatus.mockResolvedValue(status());
  renderAt("/login?error=oidc_mismatch&error_description=%3Cscript%3E");

  expect(await screen.findByText(LOGIN_ERRORS.oidc_mismatch)).toBeVisible();
  expect(screen.queryByText("<script>")).toBeNull();
});

test("offers each enabled provider with next passed through", async () => {
  getAuthStatus.mockResolvedValue(
    status({
      login: {
        oidc: { label: "Authentik", autoRedirect: false },
        strava: true,
      },
    }),
  );
  renderAt("/login?next=%2Fhevy");

  expect(await screen.findByRole("link", { name: "Sign in with Authentik" })).toHaveAttribute(
    "href",
    "/auth/oidc/login?intent=login&next=%2Fhevy",
  );
  expect(screen.getByRole("link", { name: "Sign in with Strava" })).toHaveAttribute(
    "href",
    "/auth/strava/login?next=%2Fhevy",
  );
});

test("?local=1 shows the password form and skips auto-redirect", async () => {
  const assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, assign });
  getAuthStatus.mockResolvedValue(
    status({
      login: { oidc: { label: "Authentik", autoRedirect: true }, strava: false },
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
      login: { oidc: { label: "Authentik", autoRedirect: true }, strava: false },
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
