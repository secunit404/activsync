import { Activity, Dumbbell, Settings, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { NavLink } from "react-router";

type RailDestination = {
  to: string;
  label: string;
  icon: LucideIcon;
};

const destinations: RailDestination[] = [
  { to: "/", label: "Activities", icon: Activity },
  { to: "/hevy", label: "Hevy", icon: Dumbbell },
  { to: "/settings", label: "Settings", icon: Settings },
];

/**
 * Persistent 74px desktop icon rail. Nav only — no data props — so it unit
 * tests without a QueryClientProvider. `children` is a footer slot the
 * layout route fills with <AppVersionFooter>, since that needs query data
 * the rail itself must not depend on.
 */
export function AppRail({ children }: { children?: ReactNode }) {
  return (
    <nav
      data-testid="app-rail"
      aria-label="Primary"
      className="fixed inset-y-0 left-0 z-20 hidden w-[74px] flex-col items-center gap-[26px] border-r border-border bg-rail py-[22px] md:flex"
    >
      <img
        src="/favicon.ico"
        alt="ActivSync"
        className="size-[38px] rounded-[11px] object-cover"
      />
      <ul className="flex flex-col items-center gap-2">
        {destinations.map(({ to, label, icon: Icon }) => (
          <li key={to}>
            <NavLink
              to={to}
              aria-label={label}
              className="grid size-[42px] place-items-center rounded-lg border border-transparent text-muted-foreground transition-colors hover:text-foreground aria-[current=page]:border-border aria-[current=page]:bg-popover aria-[current=page]:text-primary"
            >
              <Icon className="size-[19px]" aria-hidden="true" />
            </NavLink>
          </li>
        ))}
      </ul>
      {children}
    </nav>
  );
}
