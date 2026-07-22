import { layout, route, type RouteConfig } from "@react-router/dev/routes";

export default [
  layout("./routes/app-layout.tsx", [
    route("/", "./routes/activities.tsx", [
      route(":id", "./routes/activity-detail.tsx"),
    ]),
    route("hevy", "./routes/hevy.tsx", [
      route("mapping/:templateId", "./routes/hevy-mapping.tsx"),
      route("mappings", "./routes/hevy-mappings.tsx"),
      // Mapping nests UNDER backfill so the backfill overlay stays mounted
      // beneath the editor — its preview result and selection are component
      // state, and a sibling route would unmount them.
      route("backfill", "./routes/hevy-backfill.tsx", [
        route("mapping/:templateId", "./routes/hevy-mapping.tsx", {
          id: "backfill-mapping",
        }),
      ]),
    ]),
    route("settings", "./routes/settings.tsx"),
  ]),
  route("setup", "./routes/setup.tsx"),
] satisfies RouteConfig;
