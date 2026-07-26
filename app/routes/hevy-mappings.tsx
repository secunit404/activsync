import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ChevronDown, XIcon } from "lucide-react";
import { Outlet } from "react-router";

import { ConnectionError } from "@/components/connection-error";
import { HevySectionNav } from "@/components/hevy-section-nav";
import { MappingListRow } from "@/components/mapping-list-row";
import { PageContainer, PageHeader } from "@/components/page-header";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { ApiError, getHevyTools, type HevyToolsState } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import type { Route } from "./+types/hevy-mappings";

type Mapping = HevyToolsState["mappings"][number];
type MappingFilter = "all" | "needs" | "mapped";

const FILTERS: Array<{ value: MappingFilter; label: string }> = [
  { value: "all", label: "All" },
  { value: "needs", label: "Needs mapping" },
  { value: "mapped", label: "Mapped" },
];

export function meta(): Route.MetaDescriptors {
  return [
    { title: "Exercise mapping · ActivSync" },
    {
      name: "description",
      content: "Every Hevy exercise ActivSync tracks and where it lands in Garmin.",
    },
  ];
}

/** An exercise needs action when it has no mapping — the same rule
 *  `HevyMappingSummary` uses for its count. */
function needsAction(mapping: Mapping): boolean {
  return mapping.unmapped;
}

function muscleGroupLabel(value: string): string {
  if (!value.trim()) return "Other";
  const label = value.replaceAll("_", " ").toLowerCase();
  return label.slice(0, 1).toUpperCase() + label.slice(1);
}

function groupMappings(mappings: Mapping[]): Array<[string, Mapping[]]> {
  const grouped = new Map<string, Mapping[]>();
  mappings.forEach((mapping) => {
    const label = muscleGroupLabel(mapping.muscleGroup);
    grouped.set(label, [...(grouped.get(label) ?? []), mapping]);
  });

  return [...grouped.entries()]
    .sort(([left], [right]) => {
      if (left === "Other") return 1;
      if (right === "Other") return -1;
      return left.localeCompare(right);
    })
    .map(([label, rows]) => [
      label,
      [...rows].sort((left, right) => left.title.localeCompare(right.title)),
    ]);
}

/**
 * The full mapping list. `HevyMappingSummary` on the hub only ever shows
 * exercises still needing action, which left no way to review or change one
 * already mapped — this is that view.
 *
 * No backend work was needed: `getHevyTools` already returns mapped and
 * unmapped rows alike. `view.hevy_mappings_view` filters to "rows a user can
 * act on" (custom templates, templates with a user mapping, and templates no
 * built-in table resolves), not to "rows needing action" — built-in
 * exercises Garmin resolves on its own are the ones it omits, and those have
 * nothing to configure.
 */
export default function HevyMappings() {
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<MappingFilter>("all");
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(
    () => new Set(),
  );
  const tools = useQuery({
    queryKey: queryKeys.hevyTools,
    queryFn: ({ signal }) => getHevyTools(signal),
  });

  if (tools.isPending) {
    return (
      <div className="grid min-h-64 place-items-center p-16 text-muted-foreground">
        <Spinner className="size-6" aria-label="Loading exercise mappings" />
      </div>
    );
  }

  if (tools.isError) {
    const apiError = tools.error instanceof ApiError ? tools.error : undefined;
    return (
      <PageContainer className="py-8">
        <ConnectionError
          status={apiError?.status}
          detail={apiError?.message}
          onRetry={() => tools.refetch()}
        />
      </PageContainer>
    );
  }

  const term = search.trim().toLowerCase();
  const visible = tools.data.mappings.filter((mapping) => {
    if (filter === "needs" && !needsAction(mapping)) return false;
    if (filter === "mapped" && needsAction(mapping)) return false;
    return term === "" || mapping.title.toLowerCase().includes(term);
  });
  const groups = groupMappings(visible);

  function toggleGroup(group: string) {
    setCollapsedGroups((current) => {
      const next = new Set(current);
      if (next.has(group)) next.delete(group);
      else next.add(group);
      return next;
    });
  }

  return (
    <PageContainer className="grid gap-6 py-8 md:gap-7 md:py-10">
      <PageHeader
        title="Exercise mapping"
        description="Every Hevy exercise ActivSync tracks, and where its sets and reps land in Garmin."
      />

      <HevySectionNav />

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div className="relative w-full sm:max-w-xs">
          <Input
            aria-label="Search exercises"
            placeholder="Search exercises…"
            className="h-11 pr-10"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
          {search ? (
            <button
              type="button"
              aria-label="Clear search"
              className="absolute inset-y-0 right-0 grid w-10 place-items-center text-muted-foreground transition-colors hover:text-foreground focus-visible:rounded-r-lg focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
              onClick={() => setSearch("")}
            >
              <XIcon className="size-4" aria-hidden="true" />
            </button>
          ) : null}
        </div>
        <div
          role="group"
          aria-label="Filter exercises"
          className="flex gap-1.5 rounded-xl border border-border p-1.5"
        >
          {FILTERS.map((option) => (
            <button
              key={option.value}
              type="button"
              aria-pressed={filter === option.value}
              className={cn(
                "rounded-lg px-3.5 py-2 text-[13px] font-medium whitespace-nowrap transition-colors",
                filter === option.value
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={() => setFilter(option.value)}
            >
              {option.label}
            </button>
          ))}
        </div>
      </div>

      <Card className="gap-0 py-0">
        <CardContent className="p-0">
          {visible.length === 0 ? (
            <p className="px-5 py-8 text-sm text-muted-foreground">
              No exercises match this search and filter.
            </p>
          ) : (
            <div>
              {groups.map(([group, mappings]) => {
                const headingId = `muscle-${group.toLowerCase().replaceAll(" ", "-")}`;
                const contentId = `${headingId}-exercises`;
                // A search should never hide a matching exercise inside a
                // collapsed group. Clearing it restores the user's choice.
                const collapsed = term === "" && collapsedGroups.has(group);
                return (
                  <section key={group} aria-labelledby={headingId}>
                    <div className="sticky top-0 z-10 border-b border-border bg-muted">
                      <button
                        type="button"
                        aria-expanded={!collapsed}
                        aria-controls={contentId}
                        className="flex w-full items-center gap-3 px-5 py-3.5 text-left transition-colors hover:bg-foreground/[0.04] focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-inset focus-visible:ring-ring/50"
                        onClick={() => toggleGroup(group)}
                      >
                        <ChevronDown
                          className={cn(
                            "size-4 shrink-0 text-muted-foreground transition-transform",
                            collapsed ? "-rotate-90" : "rotate-0",
                          )}
                          aria-hidden="true"
                        />
                        <h2
                          id={headingId}
                          className="flex-1 font-mono text-xs font-bold tracking-[0.07em] text-foreground uppercase"
                        >
                          {group}
                        </h2>
                        <span className="font-mono text-xs text-muted-foreground">
                          {mappings.length}
                        </span>
                      </button>
                    </div>
                    <ul id={contentId} hidden={collapsed}>
                      {mappings.map((mapping) => (
                        <MappingListRow key={mapping.templateId} mapping={mapping} />
                      ))}
                    </ul>
                  </section>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      {/* The mapping editor mounts here as an overlay, leaving this page —
          and its search and filter — mounted underneath. */}
      <Outlet />
    </PageContainer>
  );
}
