import type { ReactNode } from "react";

import { AppBrand } from "@/components/app-shell";
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
  action,
}: {
  title: string;
  description: string;
  /**
   * Right-aligned slot for a status chip or an inline loading indicator.
   * `relative` below is for actions that opt out of the flex flow entirely
   * (see the Activities spinner) — an indicator that comes and goes must not
   * be able to re-wrap the description and shift the whole page.
   */
  action?: ReactNode;
}) {
  return (
    <header className="relative flex flex-wrap items-end justify-between gap-4">
      <div className="grid min-w-0 gap-2">
        {/* The same two-tone wordmark as the rail and the wizard — the
            eyebrow used to paint it in one colour. */}
        <AppBrand className="text-xs font-bold tracking-[0.14em] uppercase" />
        <h1 className="text-4xl leading-none font-bold tracking-[-0.05em]">{title}</h1>
        <p className="max-w-2xl text-muted-foreground">{description}</p>
      </div>
      {action}
    </header>
  );
}
