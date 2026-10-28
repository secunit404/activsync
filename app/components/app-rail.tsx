import { Activity, Dumbbell, Settings, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { NavLink } from "react-router";

import { ReviewBadge, reviewNavLabel } from "@/components/review-badge";

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
 * Persistent 74px desktop icon rail. Nav only — no queries of its own — so it
 * unit tests without a QueryClientProvider. `children` is a footer slot the
 * layout route fills with <AppVersionFooter>, and `reviewCount` (the
 * Activities badge) is likewise handed in from the layout's app-state query.
 */
export function AppRail({
  children,
  reviewCount = 0,
}: {
  children?: ReactNode;
  reviewCount?: number;
}) {
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
              aria-label={to === "/" ? reviewNavLabel(label, reviewCount) : label}
              className="relative grid size-[42px] place-items-center rounded-lg border border-transparent text-muted-foreground transition-colors hover:text-foreground aria-[current=page]:border-border aria-[current=page]:bg-popover aria-[current=page]:text-primary"
            >
              <Icon className="size-[19px]" aria-hidden="true" />
              {to === "/" ? (
                <ReviewBadge count={reviewCount} className="-top-1.5 -right-1.5" />
              ) : null}
            </NavLink>
          </li>
        ))}
      </ul>
      {children}
    </nav>
  );
}
