import { layout, route, type RouteConfig } from "@react-router/dev/routes";

export default [
  layout("./routes/app-layout.tsx", [
    route("/", "./routes/activities.tsx", [
      route(":id", "./routes/activity-detail.tsx"),
    ]),
    // The hub's <Outlet /> mounts OVERLAYS over the hub page, so only
    // dialog-shaped routes may nest here. The mapping editor is mounted
    // under each screen it can be opened from, so closing it always returns
    // to that screen with its state intact — the editor itself is one
    // module (hevy-mapping.tsx) given a distinct route id per mount.
    route("hevy", "./routes/hevy.tsx", [
      route("mapping/:templateId", "./routes/hevy-mapping.tsx"),
      // Mapping nests UNDER backfill so the backfill overlay stays mounted
      // beneath the editor — its preview result and selection are component
      // state, and a sibling route would unmount them.
      route("backfill", "./routes/hevy-backfill.tsx", [
        route("mapping/:templateId", "./routes/hevy-mapping.tsx", {
          id: "backfill-mapping",
        }),
      ]),
    ]),
    // A full page, not an overlay — so it sits BESIDE /hevy rather than
    // inside it. Nested under the hub it would have rendered stacked below
    // the entire hub page.
    route("hevy/mappings", "./routes/hevy-mappings.tsx", [
      route("mapping/:templateId", "./routes/hevy-mapping.tsx", {
        id: "mappings-mapping",
      }),
    ]),
    route("settings", "./routes/settings.tsx"),
  ]),
  route("setup", "./routes/setup.tsx"),
] satisfies RouteConfig;
