import { Activity, Dumbbell, Settings, type LucideIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { NavLink, useLocation } from "react-router";

import { cn } from "@/lib/utils";

type TabDestination = {
  to: string;
  label: string;
  icon: LucideIcon;
};

const destinations: TabDestination[] = [
  { to: "/", label: "Activities", icon: Activity },
  { to: "/hevy", label: "Hevy", icon: Dumbbell },
  { to: "/settings", label: "Settings", icon: Settings },
];

/**
 * Which destination the minimized pill stands in for. `NavLink` works out its
 * own active state, but the minimized pill is not a `NavLink` (see below), so
 * it has to ask. Prefix matching, so `/hevy/mappings` still reads as Hevy;
 * `/` is the fallback because every path is prefixed by it.
 */
function activeDestination(pathname: string): TabDestination {
  return (
    destinations.find((d) => d.to !== "/" && pathname.startsWith(d.to)) ??
    destinations[0]
  );
}

/**
 * Cumulative downward travel, in px, that collapses the bar. Big enough that
 * momentum jitter and the rubber-band at the top of a list don't flip it,
 * small enough that a deliberate flick does.
 */
const collapseThreshold = 24;

/**
 * How long after a navigation scrolling is treated as the router's doing
 * rather than the user's. `<ScrollRestoration>` (root.tsx) returns a route to
 * its saved offset on arrival, which arrives as one enormous scroll event —
 * going back to a Hevy page left at 414px collapsed the bar the instant it
 * loaded, because the handler could not tell that jump from a 414px flick.
 * Long enough to cover a restore that waits on the route's data, short enough
 * that a real flick right after a tap only has to be repeated.
 */
const navigationSettleMs = 300;

/**
 * Collapse the bar on downward scroll, iOS 26's `tabBarMinimizeBehavior`
 * (`.onScrollDown`). Faithful to that behavior in what un-collapses it too:
 * returning to the top of the page does, and so does tapping the pill (see
 * the caller), but scrolling back up on its own does NOT — otherwise the bar
 * flickers in and out through a list the user is reading in both directions.
 *
 * Travel is accumulated rather than compared per event: a single scroll event
 * moves only a handful of pixels, so any threshold worth having would never
 * be crossed by one event's delta. Any upward movement resets the tally, so
 * the threshold measures one continuous downward gesture.
 *
 * Arriving on a route always shows the bar, and scrolling caused by that
 * arrival is discounted — see `navigationSettleMs`.
 */
function useCollapseOnScrollDown(pathname: string) {
  // The collapse is stored together with the route it was made on, and read
  // back only while that route is still showing. Arriving anywhere else
  // therefore derives to expanded on its own — no effect has to write state
  // to reset it, which is both fewer renders and what `set-state-in-effect`
  // is there to push you toward.
  const [minimized, setMinimized] = useState({ collapsed: false, pathname });
  const collapsed = minimized.collapsed && minimized.pathname === pathname;

  // The scroll listener is attached once and never re-created, so it cannot
  // close over `pathname` — it reads the current one from here instead.
  const route = useRef(pathname);
  const settleUntil = useRef(0);

  useEffect(() => {
    route.current = pathname;
    settleUntil.current = Date.now() + navigationSettleMs;
  }, [pathname]);

  const expand = useCallback(
    () => setMinimized({ collapsed: false, pathname: route.current }),
    [],
  );

  useEffect(() => {
    let lastY = window.scrollY;
    let downTravel = 0;

    const onScroll = () => {
      const y = window.scrollY;
      // Inside the settle window, only re-baseline. Accumulating here is what
      // made a restored scroll offset look like a gesture.
      if (Date.now() < settleUntil.current) {
        downTravel = 0;
        lastY = y;
        return;
      }
      if (y <= 0) {
        downTravel = 0;
        setMinimized({ collapsed: false, pathname: route.current });
      } else if (y > lastY) {
        downTravel += y - lastY;
        if (downTravel > collapseThreshold) {
          setMinimized({ collapsed: true, pathname: route.current });
        }
      } else if (y < lastY) {
        downTravel = 0;
      }
      lastY = y;
    };

    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  return [collapsed, expand] as const;
}

/** Pill chrome, shared by the full row and the minimized circle. */
const pillSurface =
  "rounded-full border border-border/80 bg-rail shadow-[0_10px_30px_rgb(0_0_0/45%)] supports-backdrop-filter:bg-rail/90 supports-backdrop-filter:backdrop-blur-xl";

/**
 * Crossfade timing. Shorter than a morph would need — nothing is travelling,
 * so a long duration only makes the two states overlap for longer.
 */
const crossfade =
  "transition-[opacity,transform] duration-200 ease-out motion-reduce:transition-none";

type AppTabBarProps = {
  /**
   * True while the Activities screen's bulk-action bar occupies this same
   * fixed-bottom slot (one or more rows selected). `AppLayout` is the one
   * source of truth for this — it owns the `hidden` state and threads it
   * down from the Activities route via `Outlet` context, since the tab bar
   * and the bulk bar live in different parts of the component tree but
   * must never both be visible at once.
   */
  hidden?: boolean;
};

/**
 * Mobile bottom tab bar: a floating pill, not an edge-to-edge band. It is
 * inset from all three edges (with `env(safe-area-inset-bottom)` on top of a
 * fixed 12px, so it clears the home indicator) and translucent-blurred, so
 * the list scrolling underneath stays visible instead of being guillotined
 * by an opaque bar. Nav only — no data props besides `hidden`. Yields to the
 * bulk-action bar while rows are selected.
 *
 * On downward scroll it minimizes to an icon-only circle in the bottom-right
 * corner, showing the active destination — iOS 26's `tabBarMinimizeBehavior`,
 * which collapses to the current tab rather than hiding, so it still answers
 * "where am I" while out of the way. Scrolling back to the top expands it, as
 * does tapping it, as does arriving on a new route.
 *
 * The two states are two separately-positioned pills that CROSSFADE, not one
 * pill that morphs. A morph was tried first and cannot be made smooth here:
 * the row's tabs are sized by the container, so as the container shrank
 * toward the corner the surviving tab's share of it grew, and the icon
 * tracked left before travelling right — measured at a 17px backwards dip
 * even after the flex sizing was replaced with explicit percentages. Nothing
 * moves in a crossfade, so there is nothing to wobble.
 *
 * `<nav>` is now a transparent positioning box; both children carry the pill
 * chrome. Only one is ever in the a11y tree (`invisible` removes the other),
 * so there is never a duplicate "Activities" for a locator to trip over.
 *
 * The 74px slot `AppLayout` reserves (and `--bottom-bar-height` in app.css)
 * still holds for both: 58px of pill plus the 12px+12px it floats within.
 */
export function AppTabBar({ hidden = false }: AppTabBarProps) {
  const { pathname } = useLocation();
  const [collapsed, expand] = useCollapseOnScrollDown(pathname);
  const { label: activeLabel, icon: ActiveIcon } = activeDestination(pathname);

  return (
    <nav
      data-testid="app-tab-bar"
      data-collapsed={collapsed ? "true" : undefined}
      aria-label="Primary"
      hidden={hidden}
      className={cn(
        // No chrome of its own and no pointer target: this is a box that marks
        // out where the two pills sit. `inset-x-4` spans the row's width; the
        // circle pins itself to the right end of it.
        "pointer-events-none fixed inset-x-4 bottom-[calc(0.75rem+env(safe-area-inset-bottom))] z-20 h-[58px] md:hidden",
        // `block`/`hidden` must stay mutually exclusive: Tailwind utilities
        // share one specificity tier, so having both present and letting the
        // `hidden` *attribute* (a separate, lower-priority UA-stylesheet rule)
        // fight a layout class would not reliably hide this — the class wins
        // regardless of the attribute. Only one of the two is ever applied.
        hidden ? "hidden" : "block",
      )}
    >
      {/* The full row. Scales from its bottom-right corner so it reads as
          receding into the circle rather than shrinking toward its middle. */}
      <div
        data-testid="app-tab-bar-row"
        className={cn(
          "absolute inset-0 flex origin-bottom-right items-center gap-1 px-1.5",
          pillSurface,
          crossfade,
          collapsed ? "invisible scale-90 opacity-0" : "pointer-events-auto",
        )}
      >
        {destinations.map(({ to, label, icon: Icon }) => (
          <NavLink
            key={to}
            to={to}
            aria-label={label}
            className="group flex h-[46px] flex-1 flex-row items-center justify-center gap-1.5 rounded-full text-muted-foreground transition-colors aria-[current=page]:bg-popover aria-[current=page]:text-primary"
          >
            <Icon className="size-[18px] shrink-0" aria-hidden="true" />
            <span className="truncate text-[11px] font-medium group-aria-[current=page]:font-semibold">
              {label}
            </span>
          </NavLink>
        ))}
      </div>

      {/* The minimized state. A button, not a `NavLink` to the current route:
          its job is "expand", it would otherwise be a second link with the
          same accessible name as one in the row, and a link that navigates
          nowhere is a worse promise to a screen reader than a button that
          does what it says. No inner fill either — at 58px the pill IS the
          button, and painting the active tab's `bg-popover` inside it left a
          disc within a ring that read as a heavy border. */}
      <button
        type="button"
        data-testid="app-tab-bar-minimized"
        aria-label={`Expand navigation, currently on ${activeLabel}`}
        onClick={expand}
        className={cn(
          "absolute right-0 bottom-0 grid size-[58px] origin-bottom-right place-items-center text-primary",
          pillSurface,
          crossfade,
          collapsed ? "pointer-events-auto" : "invisible scale-90 opacity-0",
        )}
      >
        <ActiveIcon className="size-[22px]" aria-hidden="true" />
      </button>
    </nav>
  );
}
