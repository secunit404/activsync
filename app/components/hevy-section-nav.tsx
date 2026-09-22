import { NavLink } from "react-router";

import { cn } from "@/lib/utils";

const destinations = [
  { to: "/hevy", label: "Overview", end: true },
  { to: "/hevy/mappings", label: "Exercise mappings", end: false },
] as const;

/** Persistent local navigation shared by the Hevy hub and mapping catalog. */
export function HevySectionNav() {
  return (
    <nav aria-label="Hevy sections" className="border-b border-border">
      <div className="flex gap-5">
        {destinations.map((destination) => (
          <NavLink
            key={destination.to}
            to={destination.to}
            end={destination.end}
            className={({ isActive }) =>
              cn(
                "-mb-px border-b-2 px-0.5 py-2.5 text-sm font-medium whitespace-nowrap transition-colors",
                isActive
                  ? "border-primary text-foreground"
                  : "border-transparent text-muted-foreground hover:text-foreground",
              )
            }
          >
            {destination.label}
          </NavLink>
        ))}
      </div>
    </nav>
  );
}
