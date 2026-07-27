import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";

import { PullToRefresh } from "./pull-to-refresh";

function renderPullToRefresh() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const invalidate = vi
    .spyOn(queryClient, "invalidateQueries")
    .mockResolvedValue(undefined);
  render(
    <QueryClientProvider client={queryClient}>
      <PullToRefresh />
    </QueryClientProvider>,
  );
  return { invalidate };
}

/**
 * jsdom builds no `TouchEvent`, so the handlers get a plain `Event` carrying
 * the one field they read. `touches` is the live finger list, which is what
 * `touchstart`/`touchmove` expose (and what a pinch would make longer than 1).
 */
function touch(type: string, clientYs: number[]) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "touches", {
    value: clientYs.map((clientY) => ({ clientY })),
  });
  return event;
}

function pull(from: number, to: number) {
  act(() => {
    window.dispatchEvent(touch("touchstart", [from]));
    window.dispatchEvent(touch("touchmove", [to]));
  });
}

function release() {
  act(() => {
    window.dispatchEvent(touch("touchend", []));
  });
}

function setScrollY(y: number) {
  Object.defineProperty(window, "scrollY", { value: y, configurable: true });
}

afterEach(() => {
  setScrollY(0);
  vi.restoreAllMocks();
});

test("stays out of sight until something is pulled", () => {
  renderPullToRefresh();
  const indicator = screen.getByTestId("pull-to-refresh");
  expect(indicator).not.toHaveAttribute("data-armed");
  expect(indicator).not.toHaveAttribute("data-refreshing");
});

// Damped to half the finger's travel, so 200px of drag arms the 70px trigger
// while 100px does not.
test("arms only once the pull passes the trigger distance", () => {
  renderPullToRefresh();

  pull(100, 200);
  expect(screen.getByTestId("pull-to-refresh")).not.toHaveAttribute("data-armed");

  pull(100, 300);
  expect(screen.getByTestId("pull-to-refresh")).toHaveAttribute("data-armed", "true");
});

test("refreshes on release once armed", async () => {
  const { invalidate } = renderPullToRefresh();

  pull(100, 300);
  release();

  expect(invalidate).toHaveBeenCalledTimes(1);
  await act(async () => {});
  expect(screen.getByTestId("pull-to-refresh")).not.toHaveAttribute("data-refreshing");
});

test("a pull too short to arm refreshes nothing", () => {
  const { invalidate } = renderPullToRefresh();

  pull(100, 180);
  release();

  expect(invalidate).not.toHaveBeenCalled();
});

// The gesture belongs to the page unless the page is already at the top —
// otherwise pulling down mid-list would hijack an ordinary scroll.
test("does not start when the page is scrolled", () => {
  const { invalidate } = renderPullToRefresh();
  setScrollY(400);

  pull(100, 300);
  release();

  expect(invalidate).not.toHaveBeenCalled();
  expect(screen.getByTestId("pull-to-refresh")).not.toHaveAttribute("data-armed");
});

// A pinch is two fingers and is not a pull.
test("ignores a multi-touch gesture", () => {
  const { invalidate } = renderPullToRefresh();

  act(() => {
    window.dispatchEvent(touch("touchstart", [100, 140]));
    window.dispatchEvent(touch("touchmove", [300, 340]));
  });
  release();

  expect(invalidate).not.toHaveBeenCalled();
});

// Dragging upward is ordinary scrolling and must be handed straight back,
// rather than half-tracked with `preventDefault` already called.
test("hands an upward drag back to the page", () => {
  renderPullToRefresh();

  act(() => {
    window.dispatchEvent(touch("touchstart", [300]));
    const move = touch("touchmove", [200]);
    window.dispatchEvent(move);
    expect(move.defaultPrevented).toBe(false);
  });

  expect(screen.getByTestId("pull-to-refresh")).not.toHaveAttribute("data-armed");
});

// Claiming the drag is what stops the page scrolling under the indicator.
test("claims the gesture while pulling", () => {
  renderPullToRefresh();

  act(() => {
    window.dispatchEvent(touch("touchstart", [100]));
    const move = touch("touchmove", [300]);
    window.dispatchEvent(move);
    expect(move.defaultPrevented).toBe(true);
  });
});
