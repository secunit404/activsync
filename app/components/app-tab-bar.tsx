import { Activity, Dumbbell, Settings, type LucideIcon } from "lucide-react";
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
 * Index of the tab the sliding indicator sits under. Prefix matching, so
 * `/hevy/mappings` still reads as Hevy; `/` is the fallback because every
 * path is prefixed by it.
 */
function activeIndex(pathname: string): number {
  const index = destinations.findIndex((d) => d.to !== "/" && pathname.startsWith(d.to));
  return index === -1 ? 0 : index;
}

type AppTabBarProps = {
  /**
   * True while the Activities screen's bulk-action bar occupies this same
   * fixed-bottom slot (one or more rows selected). `AppLayout` owns this
   * state and threads it down from the Activities route via `Outlet`
   * context, since the two bars must never both be visible at once.
   */
  hidden?: boolean;
};

/**
 * Mobile bottom tab bar: a floating, translucent capsule inset from the
 * screen edges, with a highlight that slides to the active tab. It floats
 * `env(safe-area-inset-bottom)` + 12px up so it clears the home indicator;
 * the 74px slot `AppLayout` reserves (and `--bottom-bar-height` in app.css)
 * covers the 58px capsule plus that margin.
 */
export function AppTabBar({ hidden = false }: AppTabBarProps) {
  const { pathname } = useLocation();
  const current = activeIndex(pathname);

  return (
    <nav
      data-testid="app-tab-bar"
      aria-label="Primary"
      hidden={hidden}
      className={cn(
        "fixed inset-x-4 bottom-[calc(0.75rem+env(safe-area-inset-bottom))] z-20 h-[58px] rounded-full border border-border/80 bg-rail p-1.5 shadow-[0_10px_30px_rgb(0_0_0/45%)] supports-backdrop-filter:bg-rail/80 supports-backdrop-filter:backdrop-blur-xl md:hidden",
        // Tailwind's `block` would beat the `hidden` attribute's UA rule, so
        // exactly one of the two display classes is ever applied.
        hidden ? "hidden" : "block",
      )}
    >
      <div className="relative flex h-full">
        <span
          data-testid="app-tab-bar-indicator"
          aria-hidden="true"
          className="absolute inset-y-0 left-0 w-1/3 rounded-full bg-popover transition-transform duration-300 ease-out motion-reduce:transition-none"
          style={{ transform: `translateX(${current * 100}%)` }}
        />
        {destinations.map(({ to, label, icon: Icon }) => (
          <NavLink
            key={to}
            to={to}
            aria-label={label}
            className="relative flex flex-1 flex-col items-center justify-center gap-0.5 rounded-full text-muted-foreground transition-colors aria-[current=page]:text-primary"
          >
            <Icon className="size-[18px]" aria-hidden="true" />
            <span className="text-[11px] font-medium">{label}</span>
          </NavLink>
        ))}
      </div>
    </nav>
  );
}
