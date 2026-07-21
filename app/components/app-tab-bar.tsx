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
 * Mobile bottom tab bar. Nav only — no data props besides `hidden`. Sits
 * above content and yields to the bulk-action bar while rows are selected.
 */
export function AppTabBar({ hidden = false }: AppTabBarProps) {
  return (
    <nav
      data-testid="app-tab-bar"
      aria-label="Primary"
      hidden={hidden}
      className={cn(
        "fixed inset-x-0 bottom-0 z-20 h-[74px] border-t border-border bg-rail md:hidden",
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
          className="group flex flex-1 flex-col items-center justify-center gap-1 text-muted-foreground aria-[current=page]:text-primary"
        >
          <span className="grid size-[34px] place-items-center rounded-[10px] border border-transparent group-aria-[current=page]:border-border group-aria-[current=page]:bg-popover">
            <Icon className="size-4" aria-hidden="true" />
          </span>
          <span className="text-[10px] font-medium group-aria-[current=page]:font-semibold">
            {label}
          </span>
        </NavLink>
      ))}
    </nav>
  );
}
