import { act, render, renderHook, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { Toaster } from "@/components/ui/sonner";
import { ApiError, type PublishBatch, type PublishScheduled } from "@/lib/api";
import { PUBLISH_UNDO_WINDOW_MS, usePublishQueue } from "./use-publish-queue";

const { schedulePublishActivity, schedulePublishActivities, cancelPublish, getPublishBatch } =
  vi.hoisted(() => ({
    schedulePublishActivity: vi.fn(),
    schedulePublishActivities: vi.fn(),
    cancelPublish: vi.fn(),
    getPublishBatch: vi.fn(),
  }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    schedulePublishActivity,
    schedulePublishActivities,
    cancelPublish,
    getPublishBatch,
  };
});

function scheduled(batchId: string): PublishScheduled {
  return {
    batchId,
    publishScheduledAt: "2026-07-19T12:00:05+00:00",
    remainingMs: PUBLISH_UNDO_WINDOW_MS,
    undoWindowMs: PUBLISH_UNDO_WINDOW_MS,
  };
}

const publishedTwo = {
  message: "Published 2 activities to Strava",
  severity: "success" as const,
  publishedCount: 2,
  failedCount: 0,
  blockedCount: 0,
};

function done(batchId: string, overrides: Partial<PublishBatch> = {}): PublishBatch {
  return { batchId, state: "done", remainingMs: 0, result: publishedTwo, error: null, ...overrides };
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  schedulePublishActivity.mockReset().mockResolvedValue(scheduled("one"));
  schedulePublishActivities.mockReset().mockResolvedValue(scheduled("many"));
  cancelPublish.mockReset().mockResolvedValue({ message: "Nothing was sent to Strava." });
  getPublishBatch.mockReset().mockImplementation((batchId: string) => Promise.resolve(done(batchId)));
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

test("a bulk publish is scheduled on the server at once and reports the outcome after the window", async () => {
  const { result } = renderQueue();

  act(() => result.current.publishMany([1, 2]));

  expect(await screen.findByText("Publishing 2 to Strava…")).toBeInTheDocument();
  expect(schedulePublishActivities).toHaveBeenCalledWith([1, 2]);
  expect(result.current.queued.get(1)).toBe("waiting");
  expect(screen.getByText("Sends in 5 seconds unless you undo.")).toBeInTheDocument();
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS - 100));
  expect(getPublishBatch).not.toHaveBeenCalled();

  await act(() => vi.advanceTimersByTimeAsync(100));

  expect(getPublishBatch).toHaveBeenCalledWith("many");
  expect(await screen.findByText("Published 2 activities to Strava")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("a single publish schedules through the single-activity endpoint", async () => {
  const { result } = renderQueue();

  act(() => result.current.publishOne(7));

  expect(await screen.findByText("Publishing to Strava…")).toBeInTheDocument();
  expect(schedulePublishActivity).toHaveBeenCalledWith(7);
  expect(schedulePublishActivities).not.toHaveBeenCalled();
});

test("Undo in the toast cancels the publish on the server", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  const { result } = renderQueue();

  act(() => result.current.publishOne(7));
  await user.click(await screen.findByRole("button", { name: "Undo" }));
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS * 2));

  expect(cancelPublish).toHaveBeenCalledWith([7]);
  expect(getPublishBatch).not.toHaveBeenCalled();
  expect(await screen.findByText("Not published")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("undo by activity id cancels only the batch holding that activity", async () => {
  const { result } = renderQueue();

  act(() => {
    result.current.publishMany([1, 2]);
    result.current.publishOne(3);
  });
  await act(() => vi.advanceTimersByTimeAsync(10));
  act(() => result.current.undo(2));
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS));

  expect(cancelPublish).toHaveBeenCalledTimes(1);
  expect(cancelPublish).toHaveBeenCalledWith([1, 2]);
  expect(getPublishBatch).toHaveBeenCalledWith("one");
  expect(getPublishBatch).not.toHaveBeenCalledWith("many");
});

test("Undo pressed before the server answered cancels as soon as it does", async () => {
  let resolveSchedule: (value: PublishScheduled) => void = () => {};
  schedulePublishActivity.mockReturnValue(new Promise((r) => (resolveSchedule = r)));
  const { result } = renderQueue();

  act(() => result.current.publishOne(7));
  expect(result.current.queued.get(7)).toBe("waiting");
  act(() => result.current.undo(7));
  await act(async () => resolveSchedule(scheduled("one")));

  expect(cancelPublish).toHaveBeenCalledWith([7]);
  expect(await screen.findByText("Not published")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("a failed schedule shows the error and leaves nothing queued", async () => {
  schedulePublishActivity.mockRejectedValue(new TypeError("Failed to fetch"));
  const { result } = renderQueue();

  act(() => result.current.publishOne(7));

  expect(await screen.findByText("Failed to fetch")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
  expect(screen.queryByText("Publishing to Strava…")).not.toBeInTheDocument();
});

test("a publish the server could not send shows its error", async () => {
  getPublishBatch.mockResolvedValue(
    done("one", { result: null, error: "Could not publish activity: Strava is down" }),
  );
  const { result } = renderQueue();

  act(() => result.current.publishOne(7));
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS));

  expect(
    await screen.findByText("Could not publish activity: Strava is down"),
  ).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("an undo the server refuses as too late is reported and the publish is followed through", async () => {
  cancelPublish.mockRejectedValue(
    new ApiError("Too late to undo: it is already being sent to Strava.", 409),
  );
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  const { result } = renderQueue();

  act(() => result.current.publishMany([1, 2]));
  await user.click(await screen.findByRole("button", { name: "Undo" }));

  expect(
    await screen.findByText("Too late to undo: it is already being sent to Strava."),
  ).toBeInTheDocument();
  expect(await screen.findByText("Published 2 activities to Strava")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("while the server is sending the rows read as sending and the toast offers no undo", async () => {
  getPublishBatch.mockResolvedValue({
    batchId: "many",
    state: "sending",
    remainingMs: 0,
    result: null,
    error: null,
  });
  const { result } = renderQueue();

  act(() => result.current.publishMany([1, 2]));
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS + 50));

  expect(result.current.queued.get(2)).toBe("sending");
  const sendingToast = screen.getByText("Publishing 2 to Strava…").closest("[data-sonner-toast]");
  expect(sendingToast).not.toBeNull();
  expect(within(sendingToast as HTMLElement).queryByRole("button", { name: "Undo" })).toBeNull();

  getPublishBatch.mockResolvedValue(done("many"));
  await act(() => vi.advanceTimersByTimeAsync(1000));
  expect(await screen.findByText("Published 2 activities to Strava")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("a batch undone from another device reads as not published", async () => {
  getPublishBatch.mockRejectedValue(new ApiError("Nothing is publishing under that id.", 404));
  const { result } = renderQueue();

  act(() => result.current.publishOne(7));
  await act(() => vi.advanceTimersByTimeAsync(PUBLISH_UNDO_WINDOW_MS));

  expect(await screen.findByText("Not published")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("undo for a publish this page did not start cancels it on the server", async () => {
  const { result } = renderQueue();

  act(() => result.current.undo(42));

  expect(await screen.findByText("Not published")).toBeInTheDocument();
  expect(cancelPublish).toHaveBeenCalledWith([42]);
});
