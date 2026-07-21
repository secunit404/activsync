import { useQuery } from "@tanstack/react-query";
import { Outlet } from "react-router";

import { AppRail } from "@/components/app-rail";
import { AppTabBar } from "@/components/app-tab-bar";
import { AppVersionFooter } from "@/components/app-version-footer";
import { getAppState } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

/**
 * Persistent app shell: the 74px desktop rail, the mobile bottom tab bar,
 * and the routed content between them. AppRail/AppTabBar take no data
 * props (see their unit tests); the version footer needs query data, so it
 * is composed here and passed into the rail's footer slot instead of being
 * fetched inside AppRail itself.
 */
export default function AppLayout() {
  const { data } = useQuery({
    queryKey: queryKeys.appState,
    queryFn: ({ signal }) => getAppState(signal),
  });

  return (
    <div className="min-h-screen bg-background">
      <AppRail>
        {data ? <AppVersionFooter version={data.version} update={data.update} /> : null}
      </AppRail>
      <div className="pb-[74px] md:pb-0 md:pl-[74px]">
        <Outlet />
      </div>
      <AppTabBar />
    </div>
  );
}
