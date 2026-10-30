import { cn } from "@/lib/utils";

export function reviewBadgeText(count: number): string {
  return count > 99 ? "99+" : String(count);
}

/** The nav link's accessible name, e.g. "Activities, 3 need review". */
export function reviewNavLabel(label: string, count: number): string {
  if (count <= 0) {
    return label;
  }
  return `${label}, ${count} ${count === 1 ? "needs" : "need"} review`;
}

/**
 * Amber count pinned to a nav icon's top-right corner. Decorative: the link
 * carries the count in its accessible name (`reviewNavLabel`), so this is
 * hidden from assistive tech. The ring takes the surface it sits on, which
 * separates it from the icon underneath.
 */
export function ReviewBadge({ count, className }: { count: number; className?: string }) {
  if (count <= 0) {
    return null;
  }
  return (
    <span
      aria-hidden="true"
      data-testid="review-badge"
      className={cn(
        "pointer-events-none absolute grid h-[17px] min-w-[17px] place-items-center rounded-full bg-warning px-1 font-mono text-[10px] leading-none font-bold text-background tabular-nums ring-2 ring-rail",
        className,
      )}
    >
      {reviewBadgeText(count)}
    </span>
  );
}
