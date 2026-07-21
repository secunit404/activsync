import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";

type BulkActionBarProps = {
  count: number;
  names: string[];
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
 */
export function BulkActionBar({
  count,
  names,
  onClear,
  onExclude,
  onPublish,
  busy,
  publishDisabled = false,
}: BulkActionBarProps) {
  if (count <= 0) {
    return null;
  }

  const nameList = names.join(", ");
  const publishBlocked = busy || publishDisabled;

  return (
    <div
      data-testid="bulk-action-bar"
      className={cn(
        "fixed inset-x-0 bottom-0 z-30 flex flex-col gap-2.5 border-t border-primary/30 bg-primary/[0.07] px-3.5 py-3 shadow-[0_-10px_30px_rgba(0,0,0,0.35)]",
        "md:static md:inset-auto md:z-auto md:rounded-xl md:border md:border-primary/30 md:bg-primary/[0.06] md:px-4 md:py-3 md:shadow-[0_14px_40px_rgba(0,0,0,0.4)]",
      )}
    >
      {/* Mobile: stacked rows — count/Clear, names, then the two actions. */}
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
        {nameList ? (
          <span className="truncate text-xs text-muted-foreground">{nameList}</span>
        ) : null}
        <div className="flex gap-2">
          <Button
            variant="outline"
            className="flex-1"
            onClick={onExclude}
            disabled={busy}
          >
            Exclude
          </Button>
          <Button
            className="flex-[1.4]"
            onClick={onPublish}
            disabled={publishBlocked}
            title={publishDisabled ? publishDisabledReason : undefined}
            aria-description={publishDisabled ? publishDisabledReason : undefined}
          >
            {busy ? <Spinner /> : null}
            Publish {count}
          </Button>
        </div>
      </div>

      {/* Desktop/tablet: single row, count + names on the left, actions on the right. */}
      <div className="hidden items-center justify-between gap-4 md:flex">
        <div className="flex min-w-0 items-center gap-3">
          <BulkCount count={count} />
          <span className="h-5 w-px shrink-0 bg-primary/25" aria-hidden="true" />
          <span className="truncate text-sm text-muted-foreground">{nameList}</span>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="ghost" onClick={onClear}>
            Clear
          </Button>
          <Button variant="outline" onClick={onExclude} disabled={busy}>
            Exclude
          </Button>
          <Button
            onClick={onPublish}
            disabled={publishBlocked}
            title={publishDisabled ? publishDisabledReason : undefined}
            aria-description={publishDisabled ? publishDisabledReason : undefined}
          >
            {busy ? <Spinner /> : null}
            Publish {count}
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
