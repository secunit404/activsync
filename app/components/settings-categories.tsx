import { SearchIcon } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";

import { SettingsSection } from "@/components/settings-shell";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { useSettingsAction } from "@/hooks/use-settings-action";
import { groupActivityTypesByCategory } from "@/lib/activity-type-categories";
import { refreshActivityTypes, type SettingsState } from "@/lib/api";
import { cn } from "@/lib/utils";

type ActivityType = SettingsState["activityTypes"][number];
type FilterMode = "all" | "on" | "off";

/**
 * Presentational — `selected` (the pending enabled-typeKey set) is owned by
 * the Settings route so it participates in the shared Discard/Save footer.
 * Search text and the on/off pill are local UI state; `resetToken` bumps on
 * every Discard so this component clears them too (a render-time reset,
 * matching the rest of the app rather than a `useEffect`).
 */
export function CategorySettings({
  state,
  selected,
  onChange,
  resetToken,
}: {
  state: SettingsState;
  selected: Set<string>;
  onChange: (next: Set<string>) => void;
  resetToken: number;
}) {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<FilterMode>("all");
  const [syncedResetToken, setSyncedResetToken] = useState(resetToken);
  if (resetToken !== syncedResetToken) {
    setSyncedResetToken(resetToken);
    setQuery("");
    setMode("all");
  }

  const refresh = useSettingsAction(refreshActivityTypes);

  const normalizedQuery = query.trim().toLowerCase();
  const searched = useMemo(
    () =>
      normalizedQuery
        ? state.activityTypes.filter((item) =>
            item.label.toLowerCase().includes(normalizedQuery),
          )
        : state.activityTypes,
    [normalizedQuery, state.activityTypes],
  );
  const visible = useMemo(() => {
    if (mode === "all") return searched;
    return searched.filter((item) =>
      mode === "on" ? selected.has(item.typeKey) : !selected.has(item.typeKey),
    );
  }, [mode, searched, selected]);
  const groups = useMemo(
    () => groupActivityTypesByCategory(visible),
    [visible],
  );

  const totalCount = state.activityTypes.length;
  const onCount = selected.size;
  const offCount = totalCount - onCount;

  function setMany(items: ActivityType[], checked: boolean) {
    const next = new Set(selected);
    for (const item of items) {
      if (checked) next.add(item.typeKey);
      else next.delete(item.typeKey);
    }
    onChange(next);
  }

  return (
    <SettingsSection
      id="autosync"
      title="Auto-sync by type"
      description="On types publish to Strava automatically. Off types wait for manual review."
      contentClassName="grid gap-4"
    >
      {/* `items-end`, not `items-center`: the search input sits under its own
          label, so centring the row leaves the button half a label-line high. */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <label className="grid flex-1 gap-1.5 text-sm font-medium">
          Search activity types
          <span className="relative flex items-center">
            <SearchIcon className="pointer-events-none absolute left-3 size-4 text-muted-foreground" />
            <Input
              className="h-11 pl-9"
              type="search"
              placeholder="Search Garmin activity types…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            {query ? (
              <span className="absolute right-3 flex items-center gap-2 text-xs text-muted-foreground">
                <span className="font-mono">{searched.length} matches</span>
                <button
                  type="button"
                  className="underline-offset-2 hover:text-foreground hover:underline"
                  onClick={() => setQuery("")}
                >
                  Clear
                </button>
              </span>
            ) : null}
          </span>
        </label>
        <Button
          variant="outline"
          size="xl"
          disabled={!state.connections.garmin.connected || refresh.isPending}
          onClick={() => refresh.mutate()}
        >
          {refresh.isPending ? <Spinner /> : null}
          {refresh.isPending ? "Refreshing…" : "Refresh types"}
        </Button>
      </div>

      <div role="group" aria-label="Filter by state" className="flex gap-1.5">
        <FilterPill active={mode === "all"} onClick={() => setMode("all")}>
          All {totalCount}
        </FilterPill>
        <FilterPill active={mode === "on"} onClick={() => setMode("on")}>
          On {onCount}
        </FilterPill>
        <FilterPill active={mode === "off"} onClick={() => setMode("off")}>
          Off {offCount}
        </FilterPill>
      </div>

      <div className="overlay-scroll max-h-80 overflow-y-auto rounded-lg border border-border/60">
        {groups.length ? (
          groups.map(({ category, items }) => (
            <div key={category}>
              {/* `muted`, not `card` — this band has to occlude the rows
                  sliding under it, and the list it sits in is already `card`.
                  Same treatment as the muscle-group headers in
                  `routes/hevy-mappings.tsx`. */}
              <p className="sticky top-0 z-10 bg-muted px-4 py-1.5 font-mono text-[10.5px] tracking-[.14em] text-muted-foreground uppercase">
                {category.toUpperCase()}
              </p>
              {items.map((item) => (
                <label
                  key={item.typeKey}
                  htmlFor={"type-toggle-" + item.typeKey}
                  className="flex min-h-11 cursor-pointer items-center justify-between gap-3 border-b border-border/40 px-4 py-2 text-sm last:border-b-0 hover:bg-foreground/[0.04]"
                >
                  <span>{item.label}</span>
                  <Switch
                    id={"type-toggle-" + item.typeKey}
                    checked={selected.has(item.typeKey)}
                    onCheckedChange={(checked) =>
                      setMany([item], checked === true)
                    }
                  />
                </label>
              ))}
            </div>
          ))
        ) : (
          <p className="p-4 text-sm text-muted-foreground">
            {query
              ? "No activity types match “" + query + "”."
              : "Categories aren’t loaded yet. Connect Garmin, then refresh them."}
          </p>
        )}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border/70 pt-3">
        <span className="font-mono text-xs text-muted-foreground">
          {state.activityTypes.length
            ? "Scroll for all " + totalCount + " types"
            : "Categories aren’t loaded yet. Connect Garmin, then refresh them."}
        </span>
        <div className="flex gap-3.5 text-xs font-medium">
          <button
            type="button"
            className="text-primary hover:underline"
            disabled={!searched.length}
            onClick={() => setMany(searched, true)}
          >
            Enable all matches
          </button>
          <button
            type="button"
            className="text-muted-foreground hover:text-foreground hover:underline"
            disabled={!searched.length}
            onClick={() => setMany(searched, false)}
          >
            Disable all matches
          </button>
        </div>
      </div>
    </SettingsSection>
  );
}

function FilterPill({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "rounded-full px-3.5 py-1.5 text-xs font-medium transition-colors",
        active
          ? "bg-accent text-accent-foreground"
          : "border border-border/70 text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

