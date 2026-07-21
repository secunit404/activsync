import { data } from "react-router";

import type { Route } from "./+types/activity-detail";

// `:id` (see app/routes.ts) has no built-in way to constrain its shape to
// "numeric" the way an Express-style `:id(\d+)` would — React Router route
// paths don't support inline regex. Without this guard, `:id` matches any
// segment, so a typo'd path like `/typo` would fall through to this
// placeholder instead of a 404. SPA mode (react-router.config.ts sets
// `ssr: false`) only supports `clientLoader`, not `loader`, on non-root
// routes — see the SPA guide.
export function clientLoader({ params }: Route.ClientLoaderArgs) {
  if (!/^\d+$/.test(params.id)) {
    throw data("Not Found", { status: 404, statusText: "Not Found" });
  }
  return null;
}

// Placeholder route body — Task 6 (ResponsiveOverlay) and later tasks
// replace this with the real activity details view/edit modal (desktop) /
// full-screen cover (mobile).
export default function ActivityDetail() {
  return <h1>Activity detail</h1>;
}
