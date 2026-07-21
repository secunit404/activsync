import { useSearchParams } from "react-router";

import { cn } from "@/lib/utils";
import type { PublishStatus } from "@/lib/api";

type FilterOption = { value: PublishStatus | null; label: string };

// "excluded" is intentionally omitted — the design handoff's filter row is
// All / Pending / Held / Published / Missing only.
const filterOptions: FilterOption[] = [
  { value: null, label: "All" },
  { value: "pending", label: "Pending" },
  { value: "held", label: "Held" },
  { value: "published", label: "Published" },
  { value: "missing", label: "Missing" },
];

const validStatuses = new Set<PublishStatus>([
  "pending",
  "held",
  "published",
  "missing",
  "excluded",
]);

type ActivityFilterPillsProps = {
  counts: Record<PublishStatus, number>;
};

/**
 * Reads/writes the `status` search param directly (self-contained, per the
 * task interface) rather than taking a callback — the Activities route
 * re-derives its query from the same URL on the next render. Counts render
 * next to each label on rail-based layouts (md: and up); the mobile phone
 * layout condenses to labels only, matching the design handoff.
 */
export function ActivityFilterPills({ counts }: ActivityFilterPillsProps) {
  const [searchParams, setSearchParams] = useSearchParams();
  const activeStatus = parseStatus(searchParams.get("status"));
  const totalCount = Object.values(counts).reduce((total, count) => total + count, 0);

  return (
    <div
      role="group"
      aria-label="Filter activities by status"
      className="flex flex-nowrap gap-1 rounded-xl border border-border bg-muted/30 p-1.5 md:flex-wrap md:gap-1.5"
    >
      {filterOptions.map((option) => {
        const isActive = option.value === activeStatus;
        const count = option.value === null ? totalCount : counts[option.value];
        return (
          <button
            key={option.label}
            type="button"
            aria-pressed={isActive}
            className={cn(
              "flex-1 rounded-lg px-2 py-1.5 text-center text-[11.5px] font-medium whitespace-nowrap transition-colors md:flex-none md:px-3.5 md:py-2 md:text-[13px]",
              isActive
                ? "bg-accent text-accent-foreground"
                : "text-muted-foreground hover:text-foreground",
            )}
            onClick={() => selectStatus(option.value)}
          >
            {option.label}
            <span className="ml-1 hidden font-mono md:inline">{count}</span>
          </button>
        );
      })}
    </div>
  );

  function selectStatus(status: PublishStatus | null) {
    const next = new URLSearchParams(searchParams);
    if (status === null) {
      next.delete("status");
    } else {
      next.set("status", status);
    }
    next.delete("page");
    setSearchParams(next);
  }
}

function parseStatus(raw: string | null): PublishStatus | null {
  return raw !== null && validStatuses.has(raw as PublishStatus)
    ? (raw as PublishStatus)
    : null;
}
