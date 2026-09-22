import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

export function AppBrand({ className, ...props }: ComponentProps<"span">) {
  return (
    <span
      className={cn("text-xl font-bold tracking-[-0.04em]", className)}
      aria-label="ActivSync"
      {...props}
    >
      <span className="text-[var(--activ)]">Activ</span>
      <span className="text-[var(--sync)]">Sync</span>
    </span>
  );
}
