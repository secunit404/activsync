import type { ReactNode } from "react";

import { PageContainer, PageHeader } from "@/components/page-header";
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
 * Settings page frame. Renders inside app-layout.tsx's `<Outlet />`, so the
 * rail already supplies the brand mark — this deliberately carries no brand
 * header of its own. The setup wizard, which does sit outside the rail, has
 * its own frame in `setup-step-dots.tsx` and renders `AppBrand` there; it
 * has never used this component.
 *
 * The `development` "Mock data" badge rides in the page header's action
 * slot, the same slot Hevy uses for its connection chip.
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
    <PageContainer className="min-h-screen">
      <main className="grid gap-6 py-8 md:py-10">
        <PageHeader
          title={title}
          description={description}
          action={development ? <Badge variant="outline">Mock data</Badge> : undefined}
        />
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
    </PageContainer>
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
