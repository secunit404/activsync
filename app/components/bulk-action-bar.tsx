import { useCallback } from "react";

import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";

type BulkActionBarProps = {
  /** Raw selection size — what "{n} selected" reports. */
  count: number;
  /** How many of those the server would actually accept for exclusion. */
  excludableCount: number;
  /** How many of those are in a publishable state. */
  publishableCount: number;
  onClear: () => void;
  onExclude: () => void;
  onPublish: () => void;
  busy: boolean;
  /**
   * True while the Strava connection is broken (see `AppState.connections
   * .broken`) — Publish would otherwise error immediately on submit.
   * Exclude is unaffected: it is a purely local status change, not a call
   * to Strava, so it stays enabled.
   */
  publishDisabled?: boolean;
};

const publishDisabledReason = "Reconnect Strava to resume publishing.";
const nothingExcludableReason = "None of the selected activities can be excluded.";

/**
 * Contextual action bar for the Activities screen's bulk selection (handoff
 * frame `4c`). Renders nothing while nothing is selected — it slides in the
 * moment a checkbox is checked, there is no separate "select multiple"
 * mode to enter first.
 *
 * One element in the DOM (`data-testid="bulk-action-bar"`), not two swapped
 * by viewport: `e2e/bulk-select.spec.ts` queries this testid with a single
 * (strict-mode) locator across every viewport project, so a desktop copy
 * and a mobile copy both present at once would be a duplicate match. The
 * two layouts are two inner blocks toggled by CSS breakpoint only (same
 * convention as `ActivitiesTable`/`ActivityCard`), while the outer element
 * itself stays a single node: fixed to the bottom of the viewport (in the
 * mobile tab bar's slot — see `AppTabBar`) below `md:`, static and inline
 * above the table from `md:` up.
 *
 * Below `md:`, this occupies the exact fixed-bottom slot `AppTabBar` leaves
 * behind — and, unlike the tab bar, its height isn't a fixed constant (the
 * mobile layout can wrap its action row). The mobile toast (Task 17,
 * `sonner.tsx`) needs to sit above whichever of the two is actually
 * showing, so this keeps a shared `--bottom-bar-height` CSS custom
 * property (read by `sonner.tsx`'s `mobileOffset`, defaulted to
 * `AppTabBar`'s fixed 74px in `app.css`) in sync with its own real
 * rendered height for as long as it is mounted, and restores that 74px
 * default the moment it unmounts — a plain ref callback with a cleanup
 * function (React 19), not a `useEffect`/state pair, since nothing here is
 * React state: it is a direct measurement of the DOM written straight to a
 * CSS variable, which is exactly the kind of "sync with the browser layout"
 * side effect a `useEffect` would otherwise exist for, without the extra
 * render `set-state-in-effect` would cost.
 */
export function BulkActionBar({
  count,
  excludableCount,
  publishableCount,
  onClear,
  onExclude,
  onPublish,
  busy,
  publishDisabled = false,
}: BulkActionBarProps) {
  const bottomBarHeightRef = useCallback((node: HTMLDivElement | null) => {
    if (!node) {
      return;
    }
    const syncHeight = () => {
      document.documentElement.style.setProperty(
        "--bottom-bar-height",
        `${node.offsetHeight}px`,
      );
    };
    syncHeight();
    const observer = new ResizeObserver(syncHeight);
    observer.observe(node);
    return () => {
      observer.disconnect();
      document.documentElement.style.setProperty("--bottom-bar-height", "74px");
    };
  }, []);

  if (count <= 0) {
    return null;
  }

  // A mixed selection acts on the subset the server would accept, rather
  // than blocking on one ineligible row. Each button disables only when its
  // own eligible subset is empty.
  const excludeBlocked = busy || excludableCount === 0;
  const publishBlocked = busy || publishDisabled || publishableCount === 0;

  return (
    <div
      ref={bottomBarHeightRef}
      data-testid="bulk-action-bar"
      className={cn(
        "fixed inset-x-0 bottom-0 z-30 flex flex-col gap-2.5 border-t border-primary/30 bg-primary/[0.07] px-3.5 py-3 shadow-[0_-10px_30px_rgba(0,0,0,0.35)]",
        "md:static md:inset-auto md:z-auto md:rounded-xl md:border md:border-primary/30 md:bg-primary/[0.06] md:px-4 md:py-3 md:shadow-[0_14px_40px_rgba(0,0,0,0.4)]",
      )}
    >
      {/* Mobile: stacked rows — count/Clear above the two actions. */}
      <div className="flex flex-col gap-2.5 md:hidden">
        <div className="flex items-center justify-between gap-2">
          <BulkCount count={count} />
          <button
            type="button"
            onClick={onClear}
            className="text-sm text-muted-foreground"
          >
            Clear
          </button>
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            className="flex-1"
            onClick={onExclude}
            disabled={excludeBlocked}
            title={excludableCount === 0 ? nothingExcludableReason : undefined}
          >
            Exclude {excludableCount}
          </Button>
          <Button
            className="flex-[1.4]"
            onClick={onPublish}
            disabled={publishBlocked}
            title={publishDisabled ? publishDisabledReason : undefined}
            aria-description={publishDisabled ? publishDisabledReason : undefined}
          >
            {busy ? <Spinner /> : null}
            Publish {publishableCount}
          </Button>
        </div>
      </div>

      {/* Desktop/tablet: single row, count on the left, actions on the right. */}
      <div className="hidden items-center justify-between gap-4 md:flex">
        <BulkCount count={count} />
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="ghost" onClick={onClear}>
            Clear
          </Button>
          <Button
            variant="outline"
            onClick={onExclude}
            disabled={excludeBlocked}
            title={excludableCount === 0 ? nothingExcludableReason : undefined}
          >
            Exclude {excludableCount}
          </Button>
          <Button
            onClick={onPublish}
            disabled={publishBlocked}
            title={publishDisabled ? publishDisabledReason : undefined}
            aria-description={publishDisabled ? publishDisabledReason : undefined}
          >
            {busy ? <Spinner /> : null}
            Publish {publishableCount}
          </Button>
        </div>
      </div>
    </div>
  );
}

function BulkCount({ count }: { count: number }) {
  return (
    <span className="font-mono text-sm font-semibold text-primary">
      {count} selected
    </span>
  );
}
