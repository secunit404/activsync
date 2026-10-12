import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/**
 * A run of short metadata (date, time, source…) separated by space rather
 * than a divider glyph. Empty items are dropped so callers can pass optional
 * parts inline.
 */
export function MetaList({
  items,
  className,
}: {
  items: Array<ReactNode | null | undefined | false>;
  className?: string;
}) {
  return (
    <span className={cn("inline-flex flex-wrap items-baseline gap-x-3 gap-y-0.5", className)}>
      {items.filter(Boolean).map((item, index) => (
        <span key={index}>{item}</span>
      ))}
    </span>
  );
}
