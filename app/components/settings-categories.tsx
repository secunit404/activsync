import { SearchIcon } from "lucide-react";
import { useMemo, useState } from "react";

import { SettingsSection } from "@/components/settings-shell";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useSettingsAction } from "@/hooks/use-settings-action";
import {
  refreshActivityTypes,
  saveActivityTypes,
  type SettingsState,
} from "@/lib/api";

const COLLAPSED_COUNT = 18;

export function CategorySettings({ state }: { state: SettingsState }) {
  const [selected, setSelected] = useState(
    () =>
      new Set(
        state.activityTypes
          .filter((activityType) => activityType.autosync)
          .map((activityType) => activityType.typeKey),
      ),
  );
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState(false);
  const save = useSettingsAction(saveActivityTypes);
  const refresh = useSettingsAction(refreshActivityTypes);
  const filtered = useMemo(() => {
    const normalized = query.toLowerCase().trim();
    if (!normalized) {
      return expanded
        ? state.activityTypes
        : state.activityTypes.slice(0, COLLAPSED_COUNT);
    }
    return state.activityTypes.filter((item) =>
      (item.label + " " + item.typeKey).toLowerCase().includes(normalized),
    );
  }, [expanded, query, state.activityTypes]);
  const pending = save.isPending || refresh.isPending;

  function setVisible(checked: boolean) {
    setSelected((current) => {
      const next = new Set(current);
      for (const item of filtered) {
        if (checked) next.add(item.typeKey);
        else next.delete(item.typeKey);
      }
      return next;
    });
  }

  return (
    <SettingsSection
      id="autosync"
      title="Autosync categories"
      description="Checked categories publish automatically. Unchecked categories wait for manual review."
    >
      <div className="grid gap-5">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
          <label className="grid flex-1 gap-1.5 text-sm font-medium">
            Find a category
            <span className="relative">
              <SearchIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                className="h-11 pl-9"
                type="search"
                placeholder="Search Garmin categories…"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
            </span>
          </label>
          <Button
            variant="outline"
            className="h-11"
            disabled={!state.connections.garmin.connected || pending}
            onClick={() => refresh.mutate()}
          >
            {refresh.isPending ? <Spinner /> : null}
            {refresh.isPending ? "Refreshing…" : "Refresh categories"}
          </Button>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <p className="mr-auto text-sm text-muted-foreground" role="status">
            <strong className="text-foreground">{selected.size}</strong> enabled ·{" "}
            {filtered.length} shown
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setVisible(true)}
          >
            Select shown
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setVisible(false)}
          >
            Clear shown
          </Button>
        </div>
        {state.activityTypes.length ? (
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
            {filtered.map((activityType) => (
              <label
                key={activityType.typeKey}
                className="flex min-h-11 cursor-pointer items-center gap-3 rounded-lg border px-3 py-2 text-sm hover:bg-muted/50"
              >
                <Checkbox
                  checked={selected.has(activityType.typeKey)}
                  onCheckedChange={(checked) =>
                    setSelected((current) => {
                      const next = new Set(current);
                      if (checked === true) next.add(activityType.typeKey);
                      else next.delete(activityType.typeKey);
                      return next;
                    })
                  }
                />
                <span>{activityType.label}</span>
              </label>
            ))}
          </div>
        ) : (
          <p className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
            Categories aren’t loaded yet. Connect Garmin, then refresh them.
          </p>
        )}
        {!query && state.activityTypes.length > COLLAPSED_COUNT ? (
          <Button
            variant="outline"
            className="h-11 sm:w-fit"
            aria-expanded={expanded}
            onClick={() => setExpanded((current) => !current)}
          >
            {expanded
              ? "Show fewer categories"
              : "Show all " + state.activityTypes.length + " categories"}
          </Button>
        ) : null}
        <Button
          className="h-11 sm:w-fit"
          disabled={!state.connections.garmin.connected || pending}
          onClick={() => save.mutate([...selected].sort())}
        >
          {save.isPending ? <Spinner /> : null}
          {save.isPending ? "Saving…" : "Save categories"}
        </Button>
      </div>
    </SettingsSection>
  );
}
