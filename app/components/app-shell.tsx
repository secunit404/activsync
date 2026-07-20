import type { ComponentProps } from "react";

import type { UpdateState } from "@/lib/api";
import { cn } from "@/lib/utils";

export function AppShell({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      className={cn("mx-auto min-h-screen w-full max-w-6xl px-4 sm:px-6", className)}
      {...props}
    />
  );
}

export function AppShellHeader({ className, ...props }: ComponentProps<"header">) {
  return (
    <header
      className={cn(
        "flex min-h-18 items-center justify-between gap-4 border-b border-border/70",
        className,
      )}
      {...props}
    />
  );
}

export function AppShellMain({ className, ...props }: ComponentProps<"main">) {
  return <main className={cn("py-8 sm:py-12", className)} {...props} />;
}

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

export function AppFooter({
  version,
  update,
}: {
  version: string;
  update: UpdateState;
}) {
  return (
    <footer className="flex flex-wrap items-center justify-center gap-2 border-t py-6 text-xs text-muted-foreground">
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
  );
}
