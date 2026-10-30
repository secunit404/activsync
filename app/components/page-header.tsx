import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/**
 * The shared page column. Extracted from `SettingsShell`, which had the more
 * finished treatment (centred, constrained, generous), while Activities and
 * Hevy rendered flush against the rail — two visual languages in one app.
 *
 * The rail supplies the brand mark from `md:` up. Phones have no rail, so
 * the header row carries the same mark there, at the heading's leading edge.
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
        {/* min-h-11 matches the 44px phone controls that sit in the action
            slot (the Activities sort toggle), so the mark, the heading and
            that control share one centre line. */}
        <div className="flex min-w-0 items-center gap-3 max-md:min-h-11">
          <img
            src="/favicon.ico"
            alt="ActivSync"
            className="size-8 shrink-0 rounded-[9px] object-cover md:hidden"
          />
          <h1 className="min-w-0 text-4xl leading-none font-bold tracking-[-0.05em]">{title}</h1>
        </div>
        <p className="max-w-2xl text-muted-foreground">{description}</p>
      </div>
      {action}
    </header>
  );
}
