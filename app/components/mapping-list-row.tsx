import { Link } from "react-router";

import { Button } from "@/components/ui/button";
import type { HevyToolsState } from "@/lib/api";

export type MappingRowData = HevyToolsState["mappings"][number];

/**
 * One row of `/hevy/mappings`. Unlike `HevyMappingSummary`'s row — which only
 * ever renders exercises still needing action — this covers the mapped case
 * too: it names the current Garmin destination and offers Edit.
 */
export function MappingListRow({ mapping }: { mapping: MappingRowData }) {
  const verb = mapping.garminRejected ? "Remap" : mapping.mapped ? "Edit" : "Map";
  // Both names come from the same taxonomy lookup, so they are present or
  // absent together — but guard on both rather than render a bare "›".
  const destination =
    mapping.categoryName && mapping.subcategoryName
      ? `${mapping.categoryName} › ${mapping.subcategoryName}`
      : null;
  const needsAction = mapping.unmapped || mapping.garminRejected;

  return (
    <li className="flex items-center gap-3.5 border-b border-border/50 px-5 py-3.5 last:border-b-0">
      <div className="min-w-0 flex-1">
        <p className="truncate text-[14px] font-semibold">
          {mapping.title}
          {mapping.isCustom ? (
            <span className="ml-1.5 rounded bg-muted px-1.5 py-0.5 align-middle font-mono text-[10px] text-muted-foreground">
              CUSTOM
            </span>
          ) : null}
        </p>
        {mapping.garminRejected ? (
          <p className="truncate font-mono text-xs text-warning/90">Rejected by Garmin</p>
        ) : mapping.unmapped ? (
          <p className="truncate font-mono text-xs text-warning/90">
            Unmapped{mapping.muscleGroup ? ` · ${mapping.muscleGroup}` : ""}
          </p>
        ) : (
          <p className="truncate font-mono text-xs text-muted-foreground">
            {destination ?? "Mapped"}
          </p>
        )}
      </div>
      <Button
        asChild
        variant={needsAction ? "warning" : "outline"}
        size="sm"
        className="shrink-0"
      >
        {/* Relative, so the editor mounts as a child of this page and
            closing it returns here with the search and filter intact. */}
        <Link
          to={`mapping/${encodeURIComponent(mapping.templateId)}`}
          aria-label={`${verb} ${mapping.title}`}
        >
          {verb} →
        </Link>
      </Button>
    </li>
  );
}
