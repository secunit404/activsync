import { useCallback, useEffect, useRef } from "react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
  /**
   * The page's select-all state, for the phone dock's select-all checkbox.
   * Phones show cards with no table header to hold one, so it lives here,
   * only while a selection exists: tapping a card's checkbox starts one.
   * Desktop and tablet keep theirs in the table header.
   */
  selectAll?: boolean | "indeterminate";
  onToggleAll?: () => void;
  /**
   * Where focus goes when the bar closes under it and the control it came
   * from is gone, or was never focused: Safari does not focus a checkbox on
   * click, so a mouse selection followed by Tab into the bar records no
   * origin at all.
   */
  fallbackFocus?: () => HTMLElement | null;
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
 * itself stays a single node, fixed to the bottom of the viewport at every
 * size: in the mobile tab bar's slot (see `AppTabBar`) below `md:`, and a
 * floating dock centred over the content column from `md:` up. It never takes
 * flow space, so checking a row cannot push the list down under the cursor,
 * and it stays in reach while selecting deep in a 100-row page.
 *
 * Below `md:`, this occupies the exact fixed-bottom slot `AppTabBar` leaves
 * behind — and, unlike the tab bar, its height isn't a fixed constant (the
 * mobile layout can wrap its action row). The mobile toast (Task 17,
 * `sonner.tsx`) needs to sit above whichever of the two is actually
 * showing, so this keeps a shared `--bottom-bar-height` CSS custom
 * property (read by `sonner.tsx`'s `mobileOffset`, defaulted to
 * `AppTabBar`'s fixed 74px in `app.css`) in sync with its own real
 * rendered height for as long as it is mounted, and restores that 74px
 * default the moment it unmounts. It also writes `--bulk-bar-clearance`
 * (read by `sonner.tsx`'s desktop `offset`), so a toast raised while the
 * dock is up stacks above it instead of under it. A plain ref callback with
 * a cleanup function (React 19), not a `useEffect`/state pair: nothing here
 * is React state, only a measurement of the DOM written straight to CSS
 * variables, without the extra render `set-state-in-effect` would cost.
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
  selectAll,
  onToggleAll,
  fallbackFocus,
}: BulkActionBarProps) {
  // Where focus came from before it entered the bar, so it can go back there
  // when the bar unmounts under it (Clear, Escape, a finished action) rather
  // than being dropped on <body>.
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const fallbackFocusRef = useRef(fallbackFocus);
  useEffect(() => {
    fallbackFocusRef.current = fallbackFocus;
  }, [fallbackFocus]);

  const barRef = useCallback((node: HTMLDivElement | null) => {
    if (!node) {
      return;
    }
    const root = document.documentElement.style;
    const syncHeight = () => {
      // +24 for the 12px it floats within, top and bottom — the same
      // arithmetic behind `AppTabBar`'s 74px (58 + 12 + 12).
      root.setProperty("--bottom-bar-height", `${node.offsetHeight + 24}px`);
      root.setProperty("--bulk-bar-clearance", `${node.offsetHeight + 16}px`);
    };
    syncHeight();
    const observer = new ResizeObserver(syncHeight);
    observer.observe(node);
    return () => {
      observer.disconnect();
      root.setProperty("--bottom-bar-height", "74px");
      root.removeProperty("--bulk-bar-clearance");
      if (!node.contains(document.activeElement)) {
        return;
      }
      const origin = returnFocusRef.current;
      const target = origin?.isConnected ? origin : fallbackFocusRef.current?.();
      target?.focus();
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
      ref={barRef}
      data-testid="bulk-action-bar"
      role="region"
      aria-label="Selected activities"
      onFocus={(event) => {
        const from = event.relatedTarget;
        if (from instanceof HTMLElement && from !== document.body && !event.currentTarget.contains(from)) {
          returnFocusRef.current = from;
        }
      }}
      className={cn(
        // Mobile is the same floating dock as `AppTabBar` — inset from all
        // three edges, same radius/border/shadow — since it takes over that
        // exact slot. Its fill stays opaque at every size: fixed over a
        // scrolling list, a tinted wash let every row read straight through
        // it and the bar all but disappeared.
        "fixed inset-x-4 bottom-[calc(0.75rem+env(safe-area-inset-bottom))] z-30 flex flex-col gap-2.5 rounded-[22px] border border-border/80 bg-rail px-3 py-3 shadow-[0_10px_30px_rgb(0_0_0/45%)]",
        // From `md:` it centres over the content column, clear of the 74px
        // rail: `left`/`right` pin the column and `w-fit` + auto margins
        // centre the dock inside it.
        "md:right-6 md:bottom-6 md:left-[calc(74px+1.5rem)] md:mx-auto md:w-fit md:max-w-[calc(100vw-74px-3rem)] md:rounded-xl md:border-primary/30 md:py-2.5 md:pr-2.5 md:pl-4 md:shadow-[0_14px_40px_rgba(0,0,0,0.5)]",
      )}
    >
      {/* Mobile: stacked rows — count/Clear above the two actions. */}
      <div className="flex flex-col gap-2 md:hidden">
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-3 pl-1">
            {onToggleAll ? (
              <Checkbox
                aria-label="Select all activities"
                checked={selectAll ?? false}
                onCheckedChange={onToggleAll}
              />
            ) : null}
            <BulkCount count={count} />
          </div>
          <button
            type="button"
            onClick={onClear}
            className="-my-1 -mr-1 inline-flex min-h-11 items-center rounded-lg px-3 text-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50"
          >
            Clear
          </button>
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="xl"
            className="flex-1"
            onClick={onExclude}
            disabled={excludeBlocked}
            title={excludableCount === 0 ? nothingExcludableReason : undefined}
          >
            Exclude {excludableCount}
          </Button>
          <Button
            size="xl"
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

      {/* Desktop/tablet: single row, count on the left, actions on the right.
          Tablets are touch too, so the buttons grow to 44px on a coarse
          pointer while a mouse keeps the denser 32px row. */}
      <div className="hidden items-center gap-6 md:flex">
        <BulkCount count={count} />
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="ghost" className={touchTarget} onClick={onClear}>
            Clear
          </Button>
          <Button
            variant="outline"
            className={touchTarget}
            onClick={onExclude}
            disabled={excludeBlocked}
            title={excludableCount === 0 ? nothingExcludableReason : undefined}
          >
            Exclude {excludableCount}
          </Button>
          <Button
            className={touchTarget}
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

const touchTarget = "pointer-coarse:h-11 pointer-coarse:px-4";

function BulkCount({ count }: { count: number }) {
  return (
    <span className="font-mono text-sm font-semibold whitespace-nowrap text-foreground">
      {count} selected
    </span>
  );
}
