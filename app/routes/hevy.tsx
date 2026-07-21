import { Outlet } from "react-router";

// Placeholder route body — a later task replaces this with the real Hevy
// hub (sync queue, mapping summary, backfill entry point).
export default function Hevy() {
  return (
    <div className="p-6">
      <h1>Hevy</h1>
      <Outlet />
    </div>
  );
}
