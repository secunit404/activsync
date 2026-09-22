import { act, render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { AppTabBar } from "./app-tab-bar";

function renderTabBar(hidden?: boolean, path = "/") {
  const router = createMemoryRouter(
    [
      { path: "/", element: <AppTabBar hidden={hidden} /> },
      { path: "/hevy", element: <AppTabBar hidden={hidden} /> },
      { path: "/hevy/mappings", element: <AppTabBar hidden={hidden} /> },
    ],
    { initialEntries: [path] },
  );
  render(<RouterProvider router={router} />);
}

/**
 * jsdom has no layout, so `window.scrollY` never moves on its own — the
 * component reads it inside a real `scroll` listener, so the test sets the
 * value and fires the event the same way the browser would.
 */
function scrollTo(y: number) {
  act(() => {
    Object.defineProperty(window, "scrollY", { value: y, configurable: true });
    window.dispatchEvent(new Event("scroll"));
  });
}

/**
 * Step past the settle window the component opens on arrival — on mount as
 * well as on every navigation, because a hard reload restores a scroll offset
 * exactly the way a client-side navigation does. Tests that scroll as the
 * *user* would have to get clear of it first; the two that check the window
 * itself deliberately do not call this.
 */
function settle() {
  act(() => {
    vi.advanceTimersByTime(500);
  });
}

beforeEach(() => {
  // `shouldAdvanceTime` keeps real timers ticking underneath, so React's own
  // scheduling still runs; only `Date.now` is under the test's control, which
  // is what the settle window is measured against.
  vi.useFakeTimers({ shouldAdvanceTime: true });
});

afterEach(() => {
  vi.useRealTimers();
  Object.defineProperty(window, "scrollY", { value: 0, configurable: true });
});

test("is visible by default", () => {
  renderTabBar();
  expect(screen.getByTestId("app-tab-bar")).toBeVisible();
});

// The bulk-action bar occupies this exact slot on mobile while rows are
// selected — the two must never both be visible, so `hidden` needs to
// actually remove this from the a11y tree/layout, not just look inert.
test("hides when the bulk-action bar is occupying its slot", () => {
  renderTabBar(true);
  const nav = screen.getByTestId("app-tab-bar");
  expect(nav).not.toBeVisible();
  expect(nav).toHaveClass("hidden");
  expect(nav).not.toHaveClass("block");
});

test("renders its three nav links when visible", () => {
  renderTabBar(false);
  expect(screen.getByRole("link", { name: "Activities" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Hevy" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Settings" })).toBeInTheDocument();
});

test("starts expanded", () => {
  renderTabBar();
  expect(screen.getByTestId("app-tab-bar")).not.toHaveAttribute("data-collapsed");
});

// The two states crossfade rather than morph, so "collapsed" means the row
// goes and the circle arrives. Asserted as classes, not via `toBeVisible()`:
// these are Tailwind utilities and no stylesheet is loaded under jsdom, so
// nothing would compute `visibility: hidden` here. `invisible` is the one
// that matters — it takes the whole subtree out of the a11y tree and out of
// tab order, where `opacity-0` alone would leave every tab focusable.
// e2e/shell.spec.ts checks the rendered result for real.
test("crossfades the row out and the minimized circle in", () => {
  renderTabBar(false, "/hevy");
  expect(screen.getByTestId("app-tab-bar-row")).not.toHaveClass("invisible");
  expect(screen.getByTestId("app-tab-bar-minimized")).toHaveClass("invisible");

  settle();
  scrollTo(200);

  expect(screen.getByTestId("app-tab-bar")).toHaveAttribute("data-collapsed", "true");
  expect(screen.getByTestId("app-tab-bar-row")).toHaveClass("invisible");
  expect(screen.getByTestId("app-tab-bar-minimized")).not.toHaveClass("invisible");
});

// The minimized pill still has to say where you are — it is the only thing on
// screen at that point. It carries the active destination in its accessible
// name, since the icon alone says nothing to a screen reader.
test("the minimized circle names the active destination", () => {
  renderTabBar(false, "/hevy");
  settle();
  scrollTo(200);

  expect(screen.getByTestId("app-tab-bar-minimized")).toHaveAccessibleName(
    "Expand navigation, currently on Hevy",
  );
});

// A nested route still belongs to its top-level tab — `/hevy/mappings` is
// Hevy. `NavLink` works this out for the row; the circle has to be told.
test("the minimized circle resolves a nested route to its top-level tab", () => {
  renderTabBar(false, "/hevy/mappings");
  settle();
  scrollTo(200);

  expect(screen.getByTestId("app-tab-bar-minimized")).toHaveAccessibleName(
    "Expand navigation, currently on Hevy",
  );
});

// It is a button, not a link to the route already showing: its whole job is
// to expand, and a link that navigates nowhere is a worse promise to a screen
// reader than a button that does what it says.
test("the minimized circle is a button, not a fourth link", () => {
  renderTabBar(false, "/hevy");
  settle();
  scrollTo(200);

  expect(screen.getByTestId("app-tab-bar-minimized").tagName).toBe("BUTTON");
  expect(screen.getAllByRole("link")).toHaveLength(3);
});

test("the minimized circle sits in the bottom-right corner", () => {
  renderTabBar();
  settle();
  scrollTo(200);

  expect(screen.getByTestId("app-tab-bar-minimized")).toHaveClass("right-0", "bottom-0");
});

// Arriving somewhere shows the nav — you have just navigated, so the bar has
// to be there to navigate again.
test("a navigation expands the bar", async () => {
  renderTabBar();
  settle();
  scrollTo(400);
  expect(screen.getByTestId("app-tab-bar")).toHaveAttribute("data-collapsed", "true");

  // No stylesheet under jsdom, so the row's links are still clickable while
  // collapsed — which is what lets this isolate "navigating expands it" from
  // "tapping the circle expands it", tested separately below.
  await act(async () => {
    screen.getByRole("link", { name: "Hevy" }).click();
  });

  expect(screen.getByTestId("app-tab-bar")).not.toHaveAttribute("data-collapsed");
});

// `<ScrollRestoration>` returns a route to its saved offset on arrival, which
// reaches this component as one huge scroll event. Counting it as a gesture
// collapsed the bar the instant a previously-scrolled page loaded — press
// Hevy, land on Hevy, bar already minimized.
test("ignores the scroll a navigation causes", async () => {
  renderTabBar();

  await act(async () => {
    screen.getByRole("link", { name: "Hevy" }).click();
  });
  // The restore lands immediately after the navigation, inside the settle
  // window — the same shape as jumping back to a page left 414px down.
  scrollTo(414);

  expect(screen.getByTestId("app-tab-bar")).not.toHaveAttribute("data-collapsed");
});

// The settle window must not swallow real scrolling forever — once it lapses,
// the very next downward gesture collapses as usual.
test("still collapses on a gesture after the settle window lapses", async () => {
  renderTabBar();
  await act(async () => {
    screen.getByRole("link", { name: "Hevy" }).click();
  });
  scrollTo(414);
  expect(screen.getByTestId("app-tab-bar")).not.toHaveAttribute("data-collapsed");

  settle();
  scrollTo(600);

  expect(screen.getByTestId("app-tab-bar")).toHaveAttribute("data-collapsed", "true");
});

// One scroll event carries only a few pixels, so the threshold accumulates
// travel. A nudge smaller than it must not flip the bar.
test("ignores a scroll shorter than the collapse threshold", () => {
  renderTabBar();
  settle();
  scrollTo(8);
  expect(screen.getByTestId("app-tab-bar")).not.toHaveAttribute("data-collapsed");
});

test("stays collapsed when scrolling back up short of the top", () => {
  renderTabBar();
  settle();
  scrollTo(400);
  expect(screen.getByTestId("app-tab-bar")).toHaveAttribute("data-collapsed", "true");

  scrollTo(200);
  expect(screen.getByTestId("app-tab-bar")).toHaveAttribute("data-collapsed", "true");
});

test("expands again at the top of the page", () => {
  renderTabBar();
  settle();
  scrollTo(400);
  expect(screen.getByTestId("app-tab-bar")).toHaveAttribute("data-collapsed", "true");

  scrollTo(0);
  expect(screen.getByTestId("app-tab-bar")).not.toHaveAttribute("data-collapsed");
});

// The other route back: tapping the minimized pill.
test("expands when the minimized circle is tapped", () => {
  renderTabBar();
  settle();
  scrollTo(400);
  expect(screen.getByTestId("app-tab-bar")).toHaveAttribute("data-collapsed", "true");

  act(() => {
    screen.getByTestId("app-tab-bar-minimized").click();
  });

  expect(screen.getByTestId("app-tab-bar")).not.toHaveAttribute("data-collapsed");
});
