import { Skeleton } from "@/components/ui/skeleton";

const STAT_TILE_COUNT = 4;
const DESKTOP_ROW_COUNT = 5;
const MOBILE_CARD_COUNT = 4;

/**
 * First paint for the Activities route (handoff frame `4d`), shown while
 * `appState`/`activities` are `isPending`. Mirrors the real layout's
 * structure — header, stat tiles (grid on `md:`+, compact strip below),
 * filter-pill row, then table rows (desktop) / cards (mobile) — at the same
 * breakpoints `StatStrip`/`ActivitiesTable`/`ActivityCard` use, so the page
 * does not reflow once real data replaces it. `role="status"` announces the
 * loading state without needing repeated live-region updates.
 *
 * `animate-pulse` (from `ui/skeleton.tsx`) is disabled under
 * `prefers-reduced-motion` globally in `app.css`.
 */
export function ActivitiesSkeleton() {
  return (
    <div
      role="status"
      aria-label="Loading activities"
      data-testid="activities-skeleton"
      className="grid gap-6 p-6 md:gap-7 md:p-8"
    >
      <div className="grid gap-2">
        <Skeleton className="h-7 w-40" />
        <Skeleton className="h-4 w-72" />
      </div>

      <div className="hidden grid-cols-2 gap-3 md:grid lg:grid-cols-4 lg:gap-3.5">
        {Array.from({ length: STAT_TILE_COUNT }, (_, index) => (
          <div
            key={index}
            className="flex flex-col gap-2 rounded-xl border border-border bg-card px-4 py-4"
          >
            <Skeleton className="h-3 w-16" />
            <Skeleton className="h-7 w-10" />
          </div>
        ))}
      </div>
      <div className="flex divide-x divide-border rounded-xl border border-border bg-card md:hidden">
        {Array.from({ length: STAT_TILE_COUNT }, (_, index) => (
          <div key={index} className="flex flex-1 flex-col items-center gap-1.5 px-1 py-2.5">
            <Skeleton className="h-2.5 w-8" />
            <Skeleton className="h-4 w-6" />
          </div>
        ))}
      </div>

      <Skeleton className="h-12 w-full rounded-xl" />

      <div
        data-testid="activities-skeleton-rows"
        className="hidden overflow-hidden rounded-xl border border-border md:block"
      >
        {Array.from({ length: DESKTOP_ROW_COUNT }, (_, index) => (
          <div
            key={index}
            className="flex items-center gap-3.5 border-b border-border/60 px-4 py-3 last:border-b-0"
          >
            <Skeleton className="size-[17px] shrink-0 rounded-[5px]" />
            <Skeleton className="h-3.5 max-w-[220px] flex-1" />
            <Skeleton className="h-3 w-14" />
            <Skeleton className="h-5 w-16 rounded-md" />
          </div>
        ))}
      </div>
      <div data-testid="activities-skeleton-cards" className="grid gap-3 md:hidden">
        {Array.from({ length: MOBILE_CARD_COUNT }, (_, index) => (
          <div key={index} className="flex items-center gap-3 rounded-xl border border-border p-3.5">
            <Skeleton className="size-[18px] shrink-0 rounded-[5px]" />
            <div className="grid flex-1 gap-1.5">
              <Skeleton className="h-3.5 w-2/3" />
              <Skeleton className="h-3 w-1/3" />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
