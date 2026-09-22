import { CheckIcon } from "lucide-react";

import { AppBrand } from "@/components/app-shell";
import { cn } from "@/lib/utils";
import type { SetupStep } from "@/lib/api";

const STEPS: Array<{ key: SetupStep; label: string; sublabel: string }> = [
  { key: "garmin", label: "Connect Garmin", sublabel: "Sign in, MFA if needed" },
  { key: "strava", label: "Authorize Strava", sublabel: "OAuth callback" },
  { key: "hevy", label: "Hevy (optional)", sublabel: "API key" },
  { key: "syncing", label: "First sync", sublabel: "Pull recent activities" },
];

/**
 * Setup wizard progress, frame `2d`. Desktop renders the numbered step rail
 * (left pane of the wizard card); mobile renders the compact four-segment
 * bar + caption from the mobile frame. Both variants stay in the DOM at
 * every width, switched by CSS breakpoint only — the project-wide
 * convention (see `ActivitiesTable`/`ActivityCard`) — so this renders no
 * competing `aria-label`/role on either block: it is a purely visual echo
 * of the step content each panel's own heading already announces.
 */
export function SetupStepDots({ step }: { step: SetupStep | null }) {
  const activeIndex = Math.max(
    STEPS.findIndex((item) => item.key === step),
    0,
  );
  return (
    <>
      <nav
        aria-hidden="true"
        className="hidden shrink-0 flex-col justify-between border-r border-border bg-[var(--rail)] px-6 py-8 md:flex md:w-[260px]"
      >
        <div className="flex flex-col gap-9">
          <AppBrand />
          <ol className="flex flex-col gap-1.5">
            {STEPS.map((item, index) => (
              <li
                key={item.key}
                className={cn(
                  "flex items-center gap-3 rounded-[11px] px-3 py-2.5",
                  index === activeIndex && "bg-primary/8",
                )}
              >
                <span
                  className={cn(
                    "grid size-7 shrink-0 place-items-center rounded-full font-mono text-[13px] font-bold",
                    index <= activeIndex
                      ? "bg-primary text-primary-foreground"
                      : "border border-border text-muted-foreground",
                  )}
                >
                  {index < activeIndex ? (
                    <CheckIcon className="size-3.5" />
                  ) : (
                    index + 1
                  )}
                </span>
                <div className="flex flex-col">
                  <span
                    className={cn(
                      "text-sm font-semibold",
                      index === activeIndex
                        ? "text-foreground"
                        : "text-muted-foreground",
                    )}
                  >
                    {item.label}
                  </span>
                  <span className="text-[11.5px] text-muted-foreground/80">
                    {item.sublabel}
                  </span>
                </div>
              </li>
            ))}
          </ol>
        </div>
        <span className="font-mono text-[11.5px] text-muted-foreground/70">
          Step {activeIndex + 1} of {STEPS.length}
        </span>
      </nav>
      <div
        aria-hidden="true"
        className="flex flex-col gap-3.5 border-b border-border px-5 py-4 md:hidden"
      >
        <AppBrand className="text-base" />
        <div className="flex gap-1.5">
          {STEPS.map((item, index) => (
            <span
              key={item.key}
              className={cn(
                "h-1 flex-1 rounded-full",
                index <= activeIndex ? "bg-primary" : "bg-secondary",
              )}
            />
          ))}
        </div>
        <span className="font-mono text-[11.5px] text-muted-foreground">
          Step {activeIndex + 1} of {STEPS.length} · {STEPS[activeIndex].label}
        </span>
      </div>
    </>
  );
}
