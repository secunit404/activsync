import { useRef, type KeyboardEvent } from "react";
import { useSearchParams } from "react-router";

import { reviewCountOf, type ActivityView } from "@/lib/activity-view";
import type { PublishStatus } from "@/lib/api";
import { cn } from "@/lib/utils";

type Segment = { value: ActivityView; label: string; count: number };

function segmentsFor(counts: Record<PublishStatus, number>): Segment[] {
  const total = Object.values(counts).reduce((sum, count) => sum + count, 0);
  return [
    { value: "review", label: "Needs review", count: reviewCountOf(counts) },
    { value: "pending", label: "Pending", count: counts.pending },
    { value: "published", label: "Published", count: counts.published },
    { value: "excluded", label: "Excluded", count: counts.excluded },
    { value: "all", label: "All", count: total },
  ];
}

function countClass(segment: Segment): string {
  if (segment.value === "review" && segment.count > 0) {
    return "text-warning";
  }
  if (segment.value === "excluded" || segment.count === 0) {
    return "text-muted-foreground";
  }
  return "text-foreground";
}

/**
 * The status counts ARE the filters: one radio group whose options carry
 * their counts. `value` is the view on screen, which can differ from the
 * URL while the route is resolving the default. Selecting always writes an
 * explicit `status` (including `all`), so a choice survives reloads and is
 * never replaced by the default.
 */
export function ActivityStatusFilter({
  counts,
  value,
}: {
  counts: Record<PublishStatus, number>;
  value: ActivityView;
}) {
  const [searchParams, setSearchParams] = useSearchParams();
  const buttons = useRef<Array<HTMLButtonElement | null>>([]);
  const segments = segmentsFor(counts);
  const selectedIndex = segments.findIndex((segment) => segment.value === value);
  const focusableIndex = selectedIndex === -1 ? 0 : selectedIndex;

  const select = (view: ActivityView) => {
    const next = new URLSearchParams(searchParams);
    next.set("status", view);
    next.delete("page");
    setSearchParams(next);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const last = segments.length - 1;
    const target = {
      ArrowRight: index === last ? 0 : index + 1,
      ArrowDown: index === last ? 0 : index + 1,
      ArrowLeft: index === 0 ? last : index - 1,
      ArrowUp: index === 0 ? last : index - 1,
      Home: 0,
      End: last,
    }[event.key];
    if (target === undefined) {
      return;
    }
    event.preventDefault();
    buttons.current[target]?.focus();
    select(segments[target].value);
  };

  return (
    <div
      role="radiogroup"
      aria-label="Show activities"
      className="flex max-w-full gap-0.5 overflow-x-auto rounded-xl border border-border p-1 sm:inline-flex sm:gap-1"
    >
      {segments.map((segment, index) => {
        const checked = index === selectedIndex;
        return (
          <button
            key={segment.value}
            ref={(element) => {
              buttons.current[index] = element;
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={index === focusableIndex ? 0 : -1}
            onClick={() => select(segment.value)}
            onKeyDown={(event) => handleKeyDown(event, index)}
            className={cn(
              "flex min-h-11 flex-auto flex-col items-center justify-center gap-0.5 rounded-lg border border-transparent px-1.5 py-1 whitespace-nowrap transition-colors outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 sm:min-h-10 sm:flex-none sm:flex-row sm:gap-2 sm:px-3.5",
              checked
                ? "border-muted-foreground/50 bg-accent text-accent-foreground"
                : "text-muted-foreground hover:bg-foreground/[0.04] hover:text-foreground",
            )}
          >
            <span className="text-xs font-medium sm:text-[13px]">{segment.label}</span>
            <span
              className={cn(
                "font-mono text-[15px] leading-none font-bold tabular-nums sm:text-[13px]",
                countClass(segment),
              )}
            >
              {segment.count}
            </span>
          </button>
        );
      })}
    </div>
  );
}
