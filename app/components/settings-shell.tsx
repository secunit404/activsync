import type { ReactNode } from "react";
import { Link } from "react-router";

import {
  AppBrand,
  AppFooter,
  AppShell,
  AppShellHeader,
  AppShellMain,
} from "@/components/app-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";
import type { UpdateState } from "@/lib/api";

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
    <AppShell>
      <AppShellHeader>
        <div className="flex items-center gap-3">
          <AppBrand />
          {development ? <Badge variant="outline">Mock data</Badge> : null}
        </div>
        <Button asChild variant="ghost" className="h-11 px-3">
          <Link to="/">Activities</Link>
        </Button>
      </AppShellHeader>
      <AppShellMain className="grid gap-6">
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
      </AppShellMain>
      {version && update ? <AppFooter version={version} update={update} /> : null}
    </AppShell>
  );
}

export function SettingsSection({
  id,
  title,
  description,
  children,
  className,
}: {
  id?: string;
  title: string;
  description?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <Card id={id} className={cn("gap-5", className)}>
      <CardHeader>
        <CardTitle>
          <h2 className="text-xl">{title}</h2>
        </CardTitle>
        {description ? <CardDescription>{description}</CardDescription> : null}
      </CardHeader>
      <CardContent>{children}</CardContent>
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
    <div className="flex min-h-16 items-center gap-3 rounded-xl border bg-background/60 px-3 py-2.5">
      <span
        className={cn(
          "size-2.5 shrink-0 rounded-full",
          connected ? "bg-emerald-500" : "bg-muted-foreground/45",
        )}
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <p className="font-semibold">{name}</p>
        <p className="text-sm text-muted-foreground">{status}</p>
        {meta ? (
          <p className="truncate text-xs text-muted-foreground" title={meta}>
            {meta}
          </p>
        ) : null}
      </div>
      {action}
    </div>
  );
}
