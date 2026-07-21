import { cn } from "@/lib/utils";
import type { PublishStatus } from "@/lib/api";

/**
 * Tone tint per status: a 12%-opacity background of the status color behind
 * matching text, per the design handoff. `missing`/`excluded` aren't in the
 * handoff's pill row example but the table (Task 8) renders every
 * PublishStatus, so all five are covered here.
 */
const statusConfig: Record<PublishStatus, { label: string; className: string }> = {
  pending: { label: "PENDING", className: "bg-primary/12 text-primary" },
  held: { label: "HELD", className: "bg-warning/12 text-warning" },
  published: { label: "PUBLISHED", className: "bg-success/12 text-success" },
  missing: { label: "MISSING", className: "bg-destructive/12 text-destructive" },
  excluded: { label: "EXCLUDED", className: "bg-muted text-muted-foreground" },
};

export function StatusPill({ status }: { status: PublishStatus }) {
  const { label, className } = statusConfig[status];
  return (
    <span
      className={cn(
        "inline-flex w-fit items-center justify-center rounded-md px-2.5 py-1 font-mono text-[11px] font-semibold tracking-[0.02em]",
        className,
      )}
    >
      {label}
    </span>
  );
}
