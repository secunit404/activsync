import { index, route, type RouteConfig } from "@react-router/dev/routes";

export default [
  index("./routes/home.tsx"),
  route("setup", "./routes/setup.tsx"),
  route("settings", "./routes/settings.tsx"),
] satisfies RouteConfig;
