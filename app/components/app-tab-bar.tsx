import { Activity, Dumbbell, Settings, type LucideIcon } from "lucide-react";
import { NavLink } from "react-router";

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
 * The 74px slot `AppLayout` reserves (and `--bottom-bar-height` in app.css)
 * still holds: 58px of pill plus the 12px+12px it floats within.
 */
export function AppTabBar({ hidden = false }: AppTabBarProps) {
  return (
    <nav
      data-testid="app-tab-bar"
      aria-label="Primary"
      hidden={hidden}
      className={cn(
        "fixed inset-x-4 bottom-[calc(0.75rem+env(safe-area-inset-bottom))] z-20 h-[58px] items-center gap-1 rounded-full border border-border/80 bg-rail px-1.5 shadow-[0_10px_30px_rgb(0_0_0/45%)] supports-backdrop-filter:bg-rail/90 supports-backdrop-filter:backdrop-blur-xl md:hidden",
        // `flex`/`hidden` must stay mutually exclusive: Tailwind utilities
        // share one specificity tier, so having both present and letting
        // the `hidden` *attribute* (a separate, lower-priority UA-stylesheet
        // rule) fight a `flex` *class* would not reliably hide this — the
        // class wins regardless of the attribute. Only one of the two
        // classes below is ever applied.
        hidden ? "hidden" : "flex",
      )}
    >
      {destinations.map(({ to, label, icon: Icon }) => (
        <NavLink
          key={to}
          to={to}
          aria-label={label}
          className="group flex h-[46px] flex-1 flex-row items-center justify-center gap-1.5 rounded-full text-muted-foreground transition-colors aria-[current=page]:bg-popover aria-[current=page]:text-primary"
        >
          <Icon className="size-[18px]" aria-hidden="true" />
          <span className="text-[11px] font-medium group-aria-[current=page]:font-semibold">
            {label}
          </span>
        </NavLink>
      ))}
    </nav>
  );
}
