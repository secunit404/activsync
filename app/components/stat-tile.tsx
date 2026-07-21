import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

export type StatTileTone = "pending" | "held" | "published" | "neutral";

/**
 * Tone -> value color, shared with the mobile compact stat strip in
 * activities.tsx so the two layouts stay visually consistent without
 * duplicating the color mapping.
 */
export const statTileToneClass: Record<StatTileTone, string> = {
  pending: "text-primary",
  held: "text-warning",
  published: "text-success",
  neutral: "text-foreground",
};

type StatTileProps = {
  label: string;
  value: string;
  tone: StatTileTone;
} & Omit<ComponentProps<"div">, "children">;

export function StatTile({ label, value, tone, className, ...props }: StatTileProps) {
  return (
    <div
      className={cn(
        "flex flex-col gap-2 rounded-xl border border-border bg-card px-4 py-4",
        className,
      )}
      {...props}
    >
      <span className="font-mono text-[11px] font-medium tracking-[0.14em] text-muted-foreground uppercase">
        {label}
      </span>
      <span
        className={cn(
          "font-mono text-3xl leading-none font-extrabold tracking-[-0.02em]",
          statTileToneClass[tone],
        )}
      >
        {value}
      </span>
    </div>
  );
}
