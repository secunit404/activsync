import { act, render, renderHook, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { Toaster } from "@/components/ui/sonner";
import { ApiError, type HevyMatchScheduled, type HevyMatchStatus } from "@/lib/api";
import { HEVY_MATCH_UNDO_WINDOW_MS, useHevyMatchQueue } from "./use-hevy-match-queue";

const { scheduleHevyMatch, cancelHevyMatch, getHevyMatchStatus } = vi.hoisted(() => ({
  scheduleHevyMatch: vi.fn(),
  cancelHevyMatch: vi.fn(),
  getHevyMatchStatus: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, scheduleHevyMatch, cancelHevyMatch, getHevyMatchStatus };
});

function scheduledMatch(): HevyMatchScheduled {
  return {
    deadline: "2026-07-22T12:00:05+00:00",
    strategy: "merge",
    remainingMs: HEVY_MATCH_UNDO_WINDOW_MS,
    undoWindowMs: HEVY_MATCH_UNDO_WINDOW_MS,
  };
}

function status(overrides: Partial<HevyMatchStatus>): HevyMatchStatus {
  return { state: "done", message: "Merged Leg Day.", ...overrides };
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  scheduleHevyMatch.mockReset().mockResolvedValue(scheduledMatch());
  cancelHevyMatch.mockReset().mockResolvedValue({ cancelled: true });
  getHevyMatchStatus.mockReset().mockResolvedValue(status({}));
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
  return renderHook(() => useHevyMatchQueue(), { wrapper });
}

async function choose(
  result: ReturnType<typeof renderQueue>["result"],
  strategy: "merge" | "replace" | "describe" = "merge",
  hevyId = "hevy-1",
  onApplied?: () => void,
) {
  await act(async () => {
    result.current.choose(hevyId, "Leg Day", strategy, onApplied);
  });
}

test("a merge is scheduled on the server at once and the toast counts down with Undo", async () => {
  const { result } = renderQueue();

  await choose(result);

  expect(scheduleHevyMatch).toHaveBeenCalledWith("hevy-1", "merge");
  expect(await screen.findByText("Merging Leg Day into Garmin…")).toBeInTheDocument();
  expect(screen.getByText("Applies in 5 seconds unless you undo.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Undo" })).toBeInTheDocument();
  expect(result.current.queued.get("hevy-1")).toBe("waiting");
  expect(getHevyMatchStatus).not.toHaveBeenCalled();
});

test.each([
  ["replace", "Replacing Leg Day on Garmin…"],
  ["describe", "Updating Leg Day's description…"],
] as const)("the %s toast names what is about to happen", async (strategy, text) => {
  const { result } = renderQueue();

  await choose(result, strategy);

  expect(await screen.findByText(text)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Undo" }));
});

test("Undo cancels on the server, reports nothing changed and never polls", async () => {
  const { result } = renderQueue();
  await choose(result);

  await userEvent.click(await screen.findByRole("button", { name: "Undo" }));

  expect(cancelHevyMatch).toHaveBeenCalledWith("hevy-1");
  expect(await screen.findByText("Not changed")).toBeInTheDocument();
  expect(screen.getByText("Garmin was left as it was.")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
  await act(() => vi.advanceTimersByTimeAsync(HEVY_MATCH_UNDO_WINDOW_MS * 2));
  expect(getHevyMatchStatus).not.toHaveBeenCalled();
});

test("after the window it polls the status and shows the server's success message", async () => {
  const { result } = renderQueue();
  const onApplied = vi.fn();
  getHevyMatchStatus
    .mockResolvedValueOnce(status({ state: "applying", message: null }))
    .mockResolvedValue(status({}));
  await choose(result, "merge", "hevy-1", onApplied);

  await act(() => vi.advanceTimersByTimeAsync(HEVY_MATCH_UNDO_WINDOW_MS - 100));
  expect(getHevyMatchStatus).not.toHaveBeenCalled();
  await act(() => vi.advanceTimersByTimeAsync(150));
  expect(result.current.queued.get("hevy-1")).toBe("applying");
  expect(await screen.findByText("Updating Garmin…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Undo" })).not.toBeInTheDocument();

  await act(() => vi.advanceTimersByTimeAsync(1100));

  expect(await screen.findByText("Merged Leg Day.")).toBeInTheDocument();
  expect(onApplied).toHaveBeenCalledOnce();
  expect(result.current.queued.size).toBe(0);
});

test("a failed apply shows the server's message as an error", async () => {
  const { result } = renderQueue();
  getHevyMatchStatus.mockResolvedValue(
    status({ state: "failed", message: "Map the missing exercises before choosing Merge or Replace." }),
  );
  await choose(result);

  await act(() => vi.advanceTimersByTimeAsync(HEVY_MATCH_UNDO_WINDOW_MS + 50));

  expect(
    await screen.findByText("Map the missing exercises before choosing Merge or Replace."),
  ).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("a decision the server skipped says nothing changed", async () => {
  const { result } = renderQueue();
  getHevyMatchStatus.mockResolvedValue(status({ state: "skipped", message: null }));
  await choose(result);

  await act(() => vi.advanceTimersByTimeAsync(HEVY_MATCH_UNDO_WINDOW_MS + 50));

  expect(await screen.findByText("Not changed")).toBeInTheDocument();
  expect(screen.getByText("Leg Day was already handled.")).toBeInTheDocument();
});

test("a decision undone from another device (404) reports nothing changed", async () => {
  const { result } = renderQueue();
  getHevyMatchStatus.mockRejectedValue(new ApiError("No Hevy match decision.", 404));
  await choose(result);

  await act(() => vi.advanceTimersByTimeAsync(HEVY_MATCH_UNDO_WINDOW_MS + 50));

  expect(await screen.findByText("Garmin was left as it was.")).toBeInTheDocument();
  expect(result.current.queued.size).toBe(0);
});

test("a 409 on Undo means it is too late and the page follows the apply", async () => {
  const { result } = renderQueue();
  cancelHevyMatch.mockRejectedValue(
    new ApiError("Too late to undo: Garmin is already being updated.", 409),
  );
  getHevyMatchStatus.mockResolvedValue(status({ state: "applying", message: null }));
  await choose(result);

  await userEvent.click(await screen.findByRole("button", { name: "Undo" }));

  expect(
    await screen.findByText("Too late to undo: Garmin is already being updated."),
  ).toBeInTheDocument();
  await act(() => vi.advanceTimersByTimeAsync(100));
  expect(result.current.queued.get("hevy-1")).toBe("applying");
  expect(getHevyMatchStatus).toHaveBeenCalled();

  getHevyMatchStatus.mockResolvedValue(status({}));
  await act(() => vi.advanceTimersByTimeAsync(1100));
  expect(result.current.queued.size).toBe(0);
});

test("a refused schedule request shows the error and leaves no queue entry", async () => {
  const { result } = renderQueue();
  scheduleHevyMatch.mockRejectedValue(new ApiError("This workout is already being matched.", 409));

  await choose(result);

  expect(await screen.findByText("This workout is already being matched.")).toBeInTheDocument();
  await act(() => vi.advanceTimersByTimeAsync(100));
  expect(result.current.queued.size).toBe(0);
});

test("Undo for a decision this page did not start cancels it on the server", async () => {
  const { result } = renderQueue();

  await act(async () => result.current.undo("hevy-elsewhere"));

  expect(cancelHevyMatch).toHaveBeenCalledWith("hevy-elsewhere");
  expect(await screen.findByText("Not changed")).toBeInTheDocument();
  const toasts = screen.getAllByText("Not changed");
  expect(within(toasts[0].closest("li") as HTMLElement).getByText("Garmin was left as it was.")).toBeInTheDocument();
});
