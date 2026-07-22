import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/**
 * The shared page column. Extracted from `SettingsShell`, which had the more
 * finished treatment (centred, constrained, generous), while Activities and
 * Hevy rendered flush against the rail — two visual languages in one app.
 *
 * `SettingsShell` keeps its own `AppBrand` header and version footer: those
 * exist for the standalone setup wizard, which sits outside the rail layout.
 * For Activities and Hevy the rail already supplies both.
 */
export function PageContainer({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("mx-auto w-full max-w-6xl px-4 sm:px-6", className)}>
      {children}
    </div>
  );
}

export function PageHeader({
  title,
  description,
  eyebrow = "ActivSync",
  action,
}: {
  title: string;
  description: string;
  eyebrow?: string;
  /** Right-aligned slot for a status chip or an inline loading indicator. */
  action?: ReactNode;
}) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-4">
      <div className="grid min-w-0 gap-2">
        <p className="text-xs font-bold tracking-[0.14em] text-[var(--sync)] uppercase">
          {eyebrow}
        </p>
        <h1 className="text-4xl leading-none font-bold tracking-[-0.05em]">{title}</h1>
        <p className="max-w-2xl text-muted-foreground">{description}</p>
      </div>
      {action}
    </header>
  );
}
