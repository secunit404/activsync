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
];

const themeScript = `(() => {
  const saved = localStorage.getItem("activsync-theme");
  const dark = saved === "dark" || (saved !== "light" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
})()`;

export function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <Meta />
        <Links />
        <script dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body>
        {children}
        <ScrollRestoration />
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
      <Toaster position="bottom-center" richColors closeButton />
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
