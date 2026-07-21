import { Activity, Dumbbell, Settings, type LucideIcon } from "lucide-react";
import { NavLink } from "react-router";

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
 * Mobile bottom tab bar. Nav only — no data props. Sits above content and
 * yields to the bulk-action bar (Task 9), which replaces it while rows are
 * selected.
 */
export function AppTabBar() {
  return (
    <nav
      data-testid="app-tab-bar"
      aria-label="Primary"
      className="fixed inset-x-0 bottom-0 z-20 flex h-[74px] border-t border-border bg-rail md:hidden"
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
