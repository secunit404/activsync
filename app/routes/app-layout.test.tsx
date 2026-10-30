import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { AppState } from "@/lib/api";
import type { AuthStatus } from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";
import AppLayout from "./app-layout";

const { getAuthStatus, getAppState } = vi.hoisted(() => ({
  getAuthStatus: vi.fn(),
  getAppState: vi.fn(),
}));

vi.mock("@/lib/auth-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/auth-api")>();
  return { ...actual, getAuthStatus };
});

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, getAppState };
});

vi.mock("@/hooks/use-prefetch-tabs", () => ({ usePrefetchTabs: () => undefined }));

function status(overrides: Partial<AuthStatus> = {}): AuthStatus {
  return {
    authenticated: true,
    method: "password",
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
      { element: <AppLayout />, children: [{ path: "/hevy", element: <p>hevy page</p> }] },
      { path: "/login", element: <p>login page</p> },
      { path: "/setup", element: <p>setup page</p> },
    ],
    { initialEntries: [path] },
  );
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return { router, client };
}

beforeEach(() => {
  getAuthStatus.mockReset();
  getAppState.mockReset();
  getAppState.mockReturnValue(new Promise(() => {}));
});

test("a signed-out browser goes to login before any data query fires", async () => {
  getAuthStatus.mockResolvedValue(status({ authenticated: false, method: null }));
  const { router } = renderAt("/hevy?tab=queue");

  expect(await screen.findByText("login page")).toBeVisible();
  expect(router.state.location.search).toBe("?next=%2Fhevy%3Ftab%3Dqueue");
  expect(getAppState).not.toHaveBeenCalled();
});

test("an unclaimed install goes to the claim step", async () => {
  getAuthStatus.mockResolvedValue(status({ authenticated: false, method: null, ownerClaimed: false }));
  renderAt("/hevy");

  expect(await screen.findByText("setup page")).toBeVisible();
  expect(getAppState).not.toHaveBeenCalled();
});

test("a signed-in owner sees the page and a sign-out button", async () => {
  getAuthStatus.mockResolvedValue(status());
  renderAt("/hevy");

  expect(await screen.findByText("hevy page")).toBeVisible();
  expect(screen.getByRole("button", { name: "Sign out" })).toBeInTheDocument();
  expect(getAppState).toHaveBeenCalled();
});

test("proxy sign-ins get no sign-out button", async () => {
  getAuthStatus.mockResolvedValue(status({ method: "proxy", user: "andreas" }));
  renderAt("/hevy");

  expect(await screen.findByText("hevy page")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Sign out" })).toBeNull();
});

test("disabled auth shows a persistent banner", async () => {
  getAuthStatus.mockResolvedValue(status({ method: "disabled", authDisabled: true, ownerClaimed: false }));
  renderAt("/hevy");

  expect(await screen.findByText(/Authentication is disabled/)).toBeVisible();
  expect(screen.getByText("hevy page")).toBeVisible();
});

function appState(reviewCount: number): AppState {
  return {
    name: "ActivSync",
    version: "1.0.0",
    release: true,
    development: false,
    setup: { complete: true, step: null },
    connections: {
      garmin: { connected: true, status: "", meta: "", email: "" },
      strava: { connected: true, status: "", meta: "" },
      broken: [],
    },
    hevy: { enabled: false, connected: false, status: "" },
    catchUpReport: null,
    update: { latest: null, available: false, repoUrl: "", releaseUrl: "" },
    reviewCount,
  };
}

// The badge reads the same app-state query the SSE refresh and every
// activity action invalidate, so a refetch is what keeps it live.
test("the rail and tab bar badge Activities with the review count and follow refetches", async () => {
  getAuthStatus.mockResolvedValue(status());
  getAppState.mockResolvedValue(appState(3));
  const { client } = renderAt("/hevy");

  expect(
    await screen.findAllByRole("link", { name: "Activities, 3 need review" }),
  ).toHaveLength(2);

  getAppState.mockResolvedValue(appState(0));
  await act(() => client.invalidateQueries({ queryKey: queryKeys.appState }));

  expect(await screen.findAllByRole("link", { name: "Activities" })).toHaveLength(2);
  expect(screen.queryByTestId("review-badge")).toBeNull();
});
