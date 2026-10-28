import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Navigate, Outlet, useLocation } from "react-router";

import { AppRail } from "@/components/app-rail";
import { AppTabBar } from "@/components/app-tab-bar";
import { AppVersionFooter } from "@/components/app-version-footer";
import { AuthDisabledBanner } from "@/components/auth-disabled-banner";
import { PullToRefresh } from "@/components/pull-to-refresh";
import { SignOutButton } from "@/components/sign-out-button";
import { Spinner } from "@/components/ui/spinner";
import { useAuthStatus } from "@/hooks/use-auth-status";
import { usePrefetchTabs } from "@/hooks/use-prefetch-tabs";
import { getAppState, loginPath } from "@/lib/api";
import type { AuthStatus } from "@/lib/auth-api";
import { queryKeys } from "@/lib/query-keys";

/**
 * Gates every rail route on the auth status before any data query fires:
 * an unclaimed install goes to the claim step, a signed-out browser to
 * /login with this location as `next`.
 */
export default function AppLayout() {
  const status = useAuthStatus();
  const location = useLocation();

  if (status.isPending) {
    return (
      <div className="grid min-h-screen place-items-center bg-background text-muted-foreground">
        <Spinner />
      </div>
    );
  }
  if (status.isError) {
    return (
      <div className="grid min-h-screen place-items-center bg-background px-4 text-center text-sm text-muted-foreground">
        Couldn’t reach ActivSync: {status.error.message}
      </div>
    );
  }
  if (!status.data.ownerClaimed && !status.data.authDisabled) {
    return <Navigate to="/setup" replace />;
  }
  if (!status.data.authenticated) {
    return <Navigate to={loginPath(`${location.pathname}${location.search}`)} replace />;
  }
  return <SignedInLayout status={status.data} />;
}

/**
 * Persistent app shell: the 74px desktop rail, the mobile bottom tab bar,
 * and the routed content between them. AppRail takes no data props (see its
 * unit tests); the version footer and sign-out button need query data, so
 * they are composed here and passed into the rail's footer slot instead.
 *
 * `setTabBarHidden` is threaded down through `Outlet`'s `context` prop so
 * the Activities route (nested a level below, via routes.ts) can tell this
 * layout to hide `AppTabBar` while its bulk-action bar occupies the same
 * fixed-bottom slot — the two components live on opposite sides of the
 * route boundary, so a plain prop can't reach across it. `useState`'s
 * setter is referentially stable, so passing it directly as the context
 * value never causes an extra render loop.
 */
function SignedInLayout({ status }: { status: AuthStatus }) {
  const { data } = useQuery({
    queryKey: queryKeys.appState,
    queryFn: ({ signal }) => getAppState(signal),
  });
  const [tabBarHidden, setTabBarHidden] = useState(false);
  usePrefetchTabs();

  return (
    <div className="min-h-screen bg-background">
      <SkipToContent />
      <PullToRefresh />
      {status.authDisabled ? (
        <div className="md:pl-[74px]">
          <AuthDisabledBanner />
        </div>
      ) : null}
      <AppRail reviewCount={data?.reviewCount}>
        <div className="mt-auto flex flex-col items-center gap-4">
          <SignOutButton method={status.method} />
          {data ? <AppVersionFooter
              version={data.version}
              release={data.release}
              update={data.update}
            /> : null}
        </div>
      </AppRail>
      {/* The dock's 74px slot plus the home-indicator inset it now floats
          above (root.tsx's `viewport-fit=cover`) — without the env() term the
          last row of a list scrolls under the dock by exactly that inset. */}
      <main
        id={MAIN_CONTENT_ID}
        tabIndex={-1}
        className="pb-[calc(74px+env(safe-area-inset-bottom))] outline-none md:pb-0 md:pl-[74px]"
      >
        <Outlet context={setTabBarHidden} />
      </main>
      <AppTabBar hidden={tabBarHidden} reviewCount={data?.reviewCount} />
    </div>
  );
}

const MAIN_CONTENT_ID = "main-content";

/**
 * First tab stop on every signed-in page, past the rail's links. Focus is
 * moved by hand rather than by following the fragment: a hash navigation
 * would go through the router and push a history entry, and Settings
 * already reads the hash to pick a section.
 *
 * While focused it overlays rather than joins the flow, so the page never
 * shifts, and sits where it hides nothing that matters. Phones: over the
 * bottom tab bar, in the dock's own shape, since every page's top padding
 * is shallower than a chip. From `md:` up: a 32px chip in the top-left
 * corner, over the rail and the page's top padding (40px on every page),
 * clear of the heading below it.
 */
function SkipToContent() {
  return (
    <a
      href={`#${MAIN_CONTENT_ID}`}
      onClick={(event) => {
        event.preventDefault();
        document.getElementById(MAIN_CONTENT_ID)?.focus();
      }}
      className="sr-only focus:not-sr-only focus:fixed focus:inset-x-4 focus:bottom-[calc(0.75rem+env(safe-area-inset-bottom))] focus:z-50 focus:flex focus:h-[58px] focus:items-center focus:justify-center focus:rounded-[22px] focus:border focus:border-border focus:bg-popover focus:px-4 focus:text-sm focus:font-semibold focus:text-popover-foreground focus:shadow-[0_10px_30px_rgb(0_0_0/45%)] focus:outline-none focus-visible:ring-3 focus-visible:ring-ring md:focus:absolute md:focus:inset-x-auto md:focus:top-1 md:focus:bottom-auto md:focus:left-2 md:focus:h-8 md:focus:rounded-full md:focus:px-3.5 md:focus:text-[13px] md:focus:shadow-none"
    >
      Skip to content
    </a>
  );
}
