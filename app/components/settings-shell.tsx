import type { ReactNode } from "react";

import { AppBrand } from "@/components/app-shell";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";
import type { UpdateState } from "@/lib/api";

/**
 * Settings/setup page frame. This renders inside app-layout.tsx's <Outlet />
 * (settings) or standalone (setup, which sits outside the rail layout), so
 * it carries its own header/footer rather than the removed AppShell
 * centered-column helpers — those were retired in Task 5 along with the
 * old single-column route shell.
 */
export function SettingsShell({
  development,
  title,
  description,
  version,
  update,
  children,
}: {
  development: boolean;
  title: string;
  description: string;
  version?: string;
  update?: UpdateState;
  children: ReactNode;
}) {
  return (
    <div className="mx-auto min-h-screen w-full max-w-6xl px-4 sm:px-6">
      <header className="flex min-h-18 items-center justify-between gap-4 border-b border-border/70">
        <div className="flex items-center gap-3">
          <AppBrand />
          {development ? <Badge variant="outline">Mock data</Badge> : null}
        </div>
      </header>
      <main className="grid gap-6 py-8 sm:py-12">
        <header className="grid gap-2">
          <p className="text-xs font-bold tracking-[0.14em] text-[var(--sync)] uppercase">
            ActivSync
          </p>
          <h1 className="text-4xl leading-none font-bold tracking-[-0.05em]">
            {title}
          </h1>
          <p className="max-w-2xl text-muted-foreground">{description}</p>
        </header>
        {children}
      </main>
      {version && update ? (
        // Mobile-only: the rail (app-rail.tsx, visible md: and up) renders
        // its own AppVersionFooter whenever it's shown, so an unconditional
        // footer here would duplicate the version at every width the rail
        // is visible — leaving exactly one on screen at any given width.
        <footer className="flex flex-wrap items-center justify-center gap-2 border-t py-6 text-xs text-muted-foreground md:hidden">
          <span>ActivSync v{version}</span>
          <span aria-hidden="true">·</span>
          <a
            className="underline-offset-4 hover:text-foreground hover:underline"
            href={update.repoUrl}
            target="_blank"
            rel="noreferrer"
          >
            GitHub
          </a>
          {update.available && update.latest ? (
            <>
              <span aria-hidden="true">·</span>
              <a
                className="font-medium text-[var(--sync)] underline-offset-4 hover:underline"
                href={update.releaseUrl}
                target="_blank"
                rel="noreferrer"
              >
                Update available: {update.latest}
              </a>
            </>
          ) : null}
        </footer>
      ) : null}
    </div>
  );
}

export function SettingsSection({
  id,
  title,
  description,
  action,
  children,
  className,
  contentClassName,
}: {
  id?: string;
  title: string;
  description?: string;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
  contentClassName?: string;
}) {
  return (
    <Card id={id} className={cn("gap-0 py-0", className)}>
      <CardHeader className="border-b border-border/70 py-4 has-data-[slot=card-action]:grid-cols-[1fr_auto]">
        <CardTitle>
          <h2 className="text-[15px] font-bold">{title}</h2>
        </CardTitle>
        {description ? <CardDescription>{description}</CardDescription> : null}
        {action ? <CardAction>{action}</CardAction> : null}
      </CardHeader>
      <CardContent className={cn("py-5", contentClassName)}>
        {children}
      </CardContent>
    </Card>
  );
}

export function ConnectionStatus({
  name,
  connected,
  status,
  meta,
  action,
}: {
  name: string;
  connected: boolean;
  status: string;
  meta?: string;
  action?: ReactNode;
}) {
  return (
    <div
      data-testid={`connection-status-${name.toLowerCase().replace(/\s+/g, "-")}`}
      className="flex min-h-14 items-center gap-3.5 border-b border-border/50 py-3 last:border-b-0"
    >
      <span
        className={cn(
          "size-2.5 shrink-0 rounded-full",
          connected ? "bg-success" : "bg-muted-foreground/45",
        )}
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <p className="text-[14.5px] font-semibold">{name}</p>
        <p className="font-mono text-xs text-muted-foreground">{status}</p>
        {meta ? (
          <p
            className="truncate font-mono text-xs text-muted-foreground"
            title={meta}
          >
            {meta}
          </p>
        ) : null}
      </div>
      {action}
    </div>
  );
}
