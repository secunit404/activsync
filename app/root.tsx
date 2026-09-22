import { useState, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  isRouteErrorResponse,
  Links,
  Meta,
  Outlet,
  Scripts,
  ScrollRestoration,
} from "react-router";

import { useLiveRefresh } from "@/hooks/use-live-refresh";
import { Toaster } from "@/components/ui/sonner";
import type { Route } from "./+types/root";
import "./app.css";

export const links: Route.LinksFunction = () => [
  { rel: "icon", href: "/favicon.ico" },
  {
    rel: "apple-touch-icon",
    href: "/apple-touch-icon.png",
    sizes: "180x180",
    type: "image/png",
  },
];

export function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        {/*
          `viewport-fit=cover` is what makes `env(safe-area-inset-*)` report
          real values — without it every one of them resolves to 0, which is
          why the floating dock sat directly on the home indicator once the
          app was added to an iPhone home screen and ran without Safari's
          chrome. Every `env(safe-area-inset-bottom)` already in the codebase
          (app-tab-bar, bulk-action-bar, responsive-overlay) was dead until
          this landed.

          The trade is that content now extends under the status bar too, so
          `body` pays that back with a matching top inset — see app.css.
        */}
        <meta
          name="viewport"
          content="width=device-width, initial-scale=1, viewport-fit=cover"
        />
        <Meta />
        <Links />
      </head>
      <body>
        {children}
        <ScrollRestoration
          getKey={(location) => `${location.pathname}${location.search}`}
        />
        <Scripts />
      </body>
    </html>
  );
}

export function HydrateFallback() {
  return (
    <main className="app-loading" aria-label="Loading ActivSync">
      <Brand />
      <div className="loading-bar" />
    </main>
  );
}

export default function App() {
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            refetchOnWindowFocus: false,
            staleTime: 15_000,
          },
        },
      }),
  );

  return (
    <QueryClientProvider client={queryClient}>
      <LiveRefreshSubscription />
      <Outlet />
      {
        // `position="bottom-right"` is the desktop layout the frame draws;
        // sonner's own sub-600px CSS collapses it to the single, full-width
        // mobile toast regardless of this value (see sonner.tsx's
        // docstring). No `richColors` — the app's own success/info/warning/
        // error palette is applied directly in app.css instead, keyed off
        // sonner's `[data-type]` attribute, so the two systems don't need
        // to agree.
      }
      <Toaster position="bottom-right" closeButton />
    </QueryClientProvider>
  );
}

function LiveRefreshSubscription() {
  useLiveRefresh();
  return null;
}

export function ErrorBoundary({ error }: Route.ErrorBoundaryProps) {
  const message = isRouteErrorResponse(error)
    ? `${error.status} ${error.statusText}`
    : error instanceof Error
      ? error.message
      : "An unexpected error occurred";

  return (
    <main className="error-page">
      <Brand />
      <h1>ActivSync hit a snag</h1>
      <p>{message}</p>
      <a href="/">Return to activities</a>
    </main>
  );
}

function Brand() {
  return (
    <span className="brand" aria-label="ActivSync">
      <span>Activ</span>
      <span>Sync</span>
    </span>
  );
}
