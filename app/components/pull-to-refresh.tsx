import { useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";

import { cn } from "@/lib/utils";

/** Finger travel, in px, that arms a refresh. */
const triggerDistance = 70;

/**
 * How far the indicator may travel, in px. The pull is damped to half the
 * finger's travel, so this is reached after 2x this much dragging — the drag
 * keeps responding past the trigger point instead of going dead, which is what
 * tells you the gesture is still being tracked.
 */
const maxPull = 96;

/** Damping, so the indicator trails the finger rather than sticking to it. */
const resistance = 0.5;

/**
 * Pull-to-refresh for the phone, where the app is usually a home-screen icon
 * with no browser chrome and therefore no reload button. There is no platform
 * feature for this — Chrome's own gesture exists but is unavailable in an iOS
 * standalone web app, and no web API exposes it — so it is hand-rolled, which
 * is also why `app.css` sets `overscroll-behavior-y: contain`: without that,
 * the native rubber-band fights this gesture for the same drag.
 *
 * Refreshing means invalidating every query, which is the same thing the SSE
 * bus does on a server-side change (see `useLiveRefresh`) — React Query then
 * refetches whatever is actually mounted rather than this having to know.
 *
 * Not gated to a breakpoint: `touchstart` only fires on a touch device, and a
 * touchscreen laptop pulling to refresh should get exactly this.
 */
export function PullToRefresh() {
  const queryClient = useQueryClient();
  const [pull, setPull] = useState(0);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    // The gesture's own state lives here rather than in React state that the
    // handlers read back: they are attached once, so they would close over the
    // first render's values, and mirroring state into a ref during render is
    // exactly what `react-hooks/refs` forbids. React state below is written
    // for rendering only, never read by this logic.
    let startY: number | null = null;
    let distance = 0;
    let busy = false;

    const refresh = async () => {
      busy = true;
      setRefreshing(true);
      try {
        await queryClient.invalidateQueries();
      } finally {
        busy = false;
        setRefreshing(false);
      }
    };

    const onTouchStart = (event: TouchEvent) => {
      // Only from a resting position at the very top, and never a second
      // gesture on top of a refresh already running. A multi-touch gesture
      // (pinch-zoom on a chart) is not a pull.
      startY =
        window.scrollY <= 0 && !busy && event.touches.length === 1
          ? event.touches[0].clientY
          : null;
    };

    const onTouchMove = (event: TouchEvent) => {
      if (startY === null) {
        return;
      }
      const travel = event.touches[0].clientY - startY;
      if (travel <= 0 || window.scrollY > 0) {
        // Upward, or the page started scrolling for real — hand the gesture
        // back rather than half-owning it.
        startY = null;
        distance = 0;
        setPull(0);
        return;
      }
      // Claims the gesture, which is what stops the page scrolling underneath
      // the indicator. Requires the non-passive listener registered below.
      event.preventDefault();
      distance = Math.min(maxPull, travel * resistance);
      setPull(distance);
    };

    const onTouchEnd = () => {
      if (startY === null) {
        return;
      }
      startY = null;
      if (distance >= triggerDistance) {
        void refresh();
      }
      distance = 0;
      setPull(0);
    };

    // `passive: false` on the move handler only — it is the one that calls
    // `preventDefault`, and declaring the others non-passive would cost
    // scroll performance for nothing.
    window.addEventListener("touchstart", onTouchStart, { passive: true });
    window.addEventListener("touchmove", onTouchMove, { passive: false });
    window.addEventListener("touchend", onTouchEnd, { passive: true });
    window.addEventListener("touchcancel", onTouchEnd, { passive: true });
    return () => {
      window.removeEventListener("touchstart", onTouchStart);
      window.removeEventListener("touchmove", onTouchMove);
      window.removeEventListener("touchend", onTouchEnd);
      window.removeEventListener("touchcancel", onTouchEnd);
    };
  }, [queryClient]);

  const armed = pull >= triggerDistance;
  const offset = refreshing ? triggerDistance : pull;
  const active = refreshing || pull > 0;

  return (
    <div
      data-testid="pull-to-refresh"
      data-armed={armed ? "true" : undefined}
      data-refreshing={refreshing ? "true" : undefined}
      aria-hidden="true"
      className="pointer-events-none fixed inset-x-0 top-[env(safe-area-inset-top)] z-30 grid place-items-center"
    >
      <div
        className={cn(
          "grid size-10 place-items-center rounded-full border border-border/80 bg-rail text-muted-foreground shadow-[0_10px_30px_rgb(0_0_0/45%)]",
          // Only animate the spring back on release: following the finger has
          // to be immediate, or the indicator lags behind the drag.
          pull > 0 ? "transition-none" : "transition-all duration-200 ease-out",
          active ? "opacity-100" : "opacity-0",
          armed || refreshing ? "text-primary" : null,
        )}
        style={{
          // `translate3d` keeps this on the compositor, so dragging never
          // waits on layout.
          transform: `translate3d(0, ${offset}px, 0) scale(${active ? 1 : 0.8})`,
        }}
      >
        <RefreshCw
          className={cn(
            "size-[18px]",
            refreshing && "animate-spin motion-reduce:animate-none",
          )}
        />
      </div>
    </div>
  );
}
