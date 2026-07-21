import { layout, route, type RouteConfig } from "@react-router/dev/routes";

export default [
  layout("./routes/app-layout.tsx", [
    route("/", "./routes/activities.tsx", [
      route(":id", "./routes/activity-detail.tsx"),
    ]),
    route("hevy", "./routes/hevy.tsx", [
      route("mapping/:templateId", "./routes/hevy-mapping.tsx"),
      route("backfill", "./routes/hevy-backfill.tsx"),
    ]),
    route("settings", "./routes/settings.tsx"),
  ]),
  route("setup", "./routes/setup.tsx"),
] satisfies RouteConfig;
