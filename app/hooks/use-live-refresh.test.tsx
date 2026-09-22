import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { queryKeys } from "@/lib/query-keys";
import { useLiveRefresh } from "./use-live-refresh";

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

beforeEach(() => {
  MockEventSource.instances = [];
  vi.stubGlobal("EventSource", MockEventSource);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

test("invalidates app and activity queries when the server refreshes", () => {
  const queryClient = new QueryClient();
  const invalidateQueries = vi
    .spyOn(queryClient, "invalidateQueries")
    .mockResolvedValue();
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  renderHook(() => useLiveRefresh(), { wrapper });
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

test("closes the event stream when the app unmounts", () => {
  const queryClient = new QueryClient();
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  const { unmount } = renderHook(() => useLiveRefresh(), { wrapper });
  const source = MockEventSource.instances[0];
  unmount();

  expect(source.close).toHaveBeenCalledOnce();
});
