import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Outlet } from "react-router";

import { AppRail } from "@/components/app-rail";
import { AppTabBar } from "@/components/app-tab-bar";
import { AppVersionFooter } from "@/components/app-version-footer";
import { PullToRefresh } from "@/components/pull-to-refresh";
import { usePrefetchTabs } from "@/hooks/use-prefetch-tabs";
import { getAppState } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

/**
 * Persistent app shell: the 74px desktop rail, the mobile bottom tab bar,
 * and the routed content between them. AppRail takes no data props (see its
 * unit tests); the version footer needs query data, so it is composed here
 * and passed into the rail's footer slot instead of being fetched inside
 * AppRail itself.
 *
 * `setTabBarHidden` is threaded down through `Outlet`'s `context` prop so
 * the Activities route (nested a level below, via routes.ts) can tell this
 * layout to hide `AppTabBar` while its bulk-action bar occupies the same
 * fixed-bottom slot — the two components live on opposite sides of the
 * route boundary, so a plain prop can't reach across it. `useState`'s
 * setter is referentially stable, so passing it directly as the context
 * value never causes an extra render loop.
 */
export default function AppLayout() {
  const { data } = useQuery({
    queryKey: queryKeys.appState,
    queryFn: ({ signal }) => getAppState(signal),
  });
  const [tabBarHidden, setTabBarHidden] = useState(false);
  usePrefetchTabs();

  return (
    <div className="min-h-screen bg-background">
      <PullToRefresh />
      <AppRail>
        {data ? <AppVersionFooter version={data.version} update={data.update} /> : null}
      </AppRail>
      {/* The dock's 74px slot plus the home-indicator inset it now floats
          above (root.tsx's `viewport-fit=cover`) — without the env() term the
          last row of a list scrolls under the dock by exactly that inset. */}
      <div className="pb-[calc(74px+env(safe-area-inset-bottom))] md:pb-0 md:pl-[74px]">
        <Outlet context={setTabBarHidden} />
      </div>
      <AppTabBar hidden={tabBarHidden} />
    </div>
  );
}
