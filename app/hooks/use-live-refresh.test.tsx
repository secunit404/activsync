import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import type { AuthStatus } from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";
import { useLiveRefresh } from "./use-live-refresh";

const { getAuthStatus } = vi.hoisted(() => ({ getAuthStatus: vi.fn() }));

vi.mock("@/lib/auth-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/auth-api")>();
  return { ...actual, getAuthStatus };
});

class MockEventSource {
  static instances: MockEventSource[] = [];

  readonly url: string;
  readonly close = vi.fn();
  private readonly listeners = new Map<string, Set<EventListener>>();

  constructor(url: string | URL) {
    this.url = String(url);
    MockEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: EventListener) {
    const listeners = this.listeners.get(type) ?? new Set<EventListener>();
    listeners.add(listener);
    this.listeners.set(type, listeners);
  }

  removeEventListener(type: string, listener: EventListener) {
    this.listeners.get(type)?.delete(listener);
  }

  emit(type: string) {
    for (const listener of this.listeners.get(type) ?? []) {
      listener(new Event(type));
    }
  }
}

function status(authenticated: boolean): AuthStatus {
  return {
    authenticated,
    method: authenticated ? "password" : null,
    user: null,
    ownerClaimed: true,
    authDisabled: false,
    login: { oidc: null, strava: false },
  };
}

function renderLiveRefresh(queryClient = new QueryClient()) {
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return renderHook(() => useLiveRefresh(), { wrapper });
}

beforeEach(() => {
  MockEventSource.instances = [];
  vi.stubGlobal("EventSource", MockEventSource);
  getAuthStatus.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

test("invalidates app and activity queries when the server refreshes", async () => {
  getAuthStatus.mockResolvedValue(status(true));
  const queryClient = new QueryClient();
  const invalidateQueries = vi
    .spyOn(queryClient, "invalidateQueries")
    .mockResolvedValue();

  renderLiveRefresh(queryClient);
  await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
  const source = MockEventSource.instances[0];

  expect(source.url).toBe("/api/events");
  act(() => source.emit("refresh"));
  expect(invalidateQueries).toHaveBeenCalledWith({
    queryKey: queryKeys.appState,
  });
  expect(invalidateQueries).toHaveBeenCalledWith({
    queryKey: queryKeys.allActivities,
  });
});

test("closes the event stream when the app unmounts", async () => {
  getAuthStatus.mockResolvedValue(status(true));

  const { unmount } = renderLiveRefresh();
  await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
  const source = MockEventSource.instances[0];
  unmount();

  expect(source.close).toHaveBeenCalledOnce();
});

test("opens no stream while signed out", async () => {
  getAuthStatus.mockResolvedValue(status(false));

  renderLiveRefresh();
  await waitFor(() => expect(getAuthStatus).toHaveBeenCalled());

  expect(MockEventSource.instances).toHaveLength(0);
});

test("an error after the session ended goes to login instead of retrying", async () => {
  const assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, pathname: "/hevy", search: "", assign });
  getAuthStatus.mockResolvedValueOnce(status(true)).mockResolvedValue(status(false));

  renderLiveRefresh();
  await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
  act(() => MockEventSource.instances[0].emit("error"));

  await waitFor(() => expect(assign).toHaveBeenCalledWith("/login?next=%2Fhevy"));
  expect(MockEventSource.instances[0].close).toHaveBeenCalled();
});

test("an error while still signed in reopens the stream with backoff", async () => {
  getAuthStatus.mockResolvedValue(status(true));

  renderLiveRefresh();
  await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
  vi.useFakeTimers();
  await act(async () => {
    MockEventSource.instances[0].emit("error");
    await vi.advanceTimersByTimeAsync(999);
  });
  expect(MockEventSource.instances).toHaveLength(1);

  await act(async () => {
    await vi.advanceTimersByTimeAsync(1);
  });
  expect(MockEventSource.instances).toHaveLength(2);
});
