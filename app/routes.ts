import { layout, route, type RouteConfig } from "@react-router/dev/routes";

export default [
  layout("./routes/app-layout.tsx", [
    route("/", "./routes/activities.tsx", [
      route(":id", "./routes/activity-detail.tsx"),
    ]),
    // The hub's <Outlet /> mounts OVERLAYS over the hub page, so only
    // dialog-shaped routes may nest here. Backfill lives inline on the hub
    // itself (hevy-backfill-card.tsx) rather than as its own route — which
    // is also why the editor opened from a locked backfill row keeps that
    // scan alive: the hub is never unmounted.
    route("hevy", "./routes/hevy.tsx", [
      route("mapping/:templateId", "./routes/hevy-mapping.tsx"),
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
