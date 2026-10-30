import { statusDescription } from "@/lib/activity-status";
import type { PublishStatus } from "@/lib/api";
import { cn } from "@/lib/utils";

/**
 * Tone tint per status: a 12%-opacity background of the status color behind
 * matching text, per the design handoff. `missing`/`excluded` aren't in the
 * handoff's pill row example but the table (Task 8) renders every
 * PublishStatus, so all five are covered here.
 */
const statusConfig: Record<PublishStatus, { label: string; className: string }> = {
  // Neutral, not brand orange: pending needs nothing from the owner (the
  // poller publishes it), and orange is kept for the Publish action itself.
  pending: { label: "PENDING", className: "bg-secondary text-foreground" },
  held: { label: "HELD", className: "bg-warning/12 text-warning" },
  published: { label: "PUBLISHED", className: "bg-success/12 text-success" },
  missing: { label: "MISSING", className: "bg-destructive/12 text-destructive" },
  excluded: { label: "EXCLUDED", className: "bg-secondary text-muted-foreground" },
};

const publishingConfig = { label: "PUBLISHING…", className: "bg-info/12 text-info" };

/** `publishing` overrides `status` while a publish waits out its undo window or is in flight. */
export function StatusPill({
  status,
  publishing = false,
  reservePublishingWidth = false,
}: {
  status: PublishStatus;
  publishing?: boolean;
  /**
   * Sizes the pill to fit PUBLISHING… whatever it shows, so switching to it
   * and back never resizes the table column it sits in.
   */
  reservePublishingWidth?: boolean;
}) {
  const { label, className } = publishing ? publishingConfig : statusConfig[status];
  return (
    <span
      title={publishing ? undefined : statusDescription(status)}
      className={cn(
        "inline-grid w-fit shrink-0 justify-items-center rounded-md px-2.5 py-1 font-mono text-xs font-semibold tracking-[0.02em]",
        reservePublishingWidth && "after:invisible after:[grid-area:1/1] after:content-['PUBLISHING…']",
        className,
      )}
    >
      <span className="[grid-area:1/1]">{label}</span>
    </span>
  );
}
