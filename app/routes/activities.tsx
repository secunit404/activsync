import { Outlet } from "react-router";

// Placeholder route body — Task 7 replaces this with the real activities
// dashboard (stat tiles, filters, table/cards). The route tree established
// in Task 5 nests activity detail under this route so the list stays
// mounted behind the detail modal/full-screen cover.
export default function Activities() {
  return (
    <div className="p-6">
      <h1>Activities</h1>
      <Outlet />
    </div>
  );
}
