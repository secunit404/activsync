import { act, render, renderHook, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { Toaster } from "@/components/ui/sonner";
import { PUBLISH_UNDO_WINDOW_MS, usePublishQueue } from "./use-publish-queue";

const { publishActivity, publishActivities } = vi.hoisted(() => ({
  publishActivity: vi.fn(),
  publishActivities: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  editActivity: vi.fn(),
  excludeActivity: vi.fn(),
  restoreActivity: vi.fn(),
  publishActivity,
  publishActivities,
}));

const published = {
  message: "Published 2 activities to Strava",
  severity: "success" as const,
  publishedCount: 2,
  failedCount: 0,
  blockedCount: 0,
};

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  publishActivity.mockReset().mockResolvedValue(published);
  publishActivities.mockReset().mockResolvedValue(published);
});

afterEach(async () => {
  await act(() => vi.runOnlyPendingTimersAsync());
  vi.useRealTimers();
});

function renderQueue() {
  const queryClient = new QueryClient();
  render(<Toaster />);
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return renderHook(() => usePublishQueue(), { wrapper });
}

test("a bulk publish waits out the undo window before sending", async () => {
  const { result } = renderQueue();

  act(() => result.current.publishMany([1, 2]));

  expect(await screen.findByText("Publishing 2 to Strava…")).toBeInTheDocument();
  expect(result.current.queued.get(1)).toBe("waiting");
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS - 100));
  expect(publishActivities).not.toHaveBeenCalled();

  await act(() => vi.advanceTimersByTimeAsync(100));

  expect(publishActivities).toHaveBeenCalledWith([1, 2]);
  expect(await screen.findByText("Published 2 activities to Strava")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("Undo in the toast cancels a single publish before anything is sent", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  const { result } = renderQueue();

  act(() => result.current.publishOne(7));
  await user.click(await screen.findByRole("button", { name: "Undo" }));
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS * 2));

  expect(publishActivity).not.toHaveBeenCalled();
  expect(await screen.findByText("Not published")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("a single publish sends through the single-activity endpoint once the window ends", async () => {
  const { result } = renderQueue();

  act(() => result.current.publishOne(7));
  expect(await screen.findByText("Publishing to Strava…")).toBeInTheDocument();
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS));

  expect(publishActivity).toHaveBeenCalledWith(7);
  expect(publishActivities).not.toHaveBeenCalled();
});

test("undo by activity id cancels only the batch holding that activity", async () => {
  const { result } = renderQueue();

  act(() => {
    result.current.publishMany([1, 2]);
    result.current.publishOne(3);
  });
  act(() => result.current.undo(2));
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS));

  expect(publishActivities).not.toHaveBeenCalled();
  expect(publishActivity).toHaveBeenCalledWith(3);
});

test("a failed publish shows the error and leaves nothing queued", async () => {
  publishActivity.mockRejectedValue(new Error("Strava is not connected"));
  const { result } = renderQueue();

  act(() => result.current.publishOne(7));
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS));

  expect(await screen.findByText("Strava is not connected")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("while the request is in flight the rows read as sending and the toast stays up", async () => {
  let resolve: (value: typeof published) => void = () => {};
  publishActivities.mockReturnValue(new Promise((r) => (resolve = r)));
  const { result } = renderQueue();

  act(() => result.current.publishMany([1, 2]));
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS + 50));

  expect(result.current.queued.get(2)).toBe("sending");
  const sendingToast = screen.getByText("Publishing 2 to Strava…").closest("[data-sonner-toast]");
  expect(sendingToast).not.toBeNull();
  expect(within(sendingToast as HTMLElement).queryByRole("button", { name: "Undo" })).toBeNull();

  await act(async () => resolve(published));
  expect(await screen.findByText("Published 2 activities to Strava")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});
