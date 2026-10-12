import { Link } from "react-router";

import { Button } from "@/components/ui/button";
import type { HevyToolsState } from "@/lib/api";

export type MappingRowData = HevyToolsState["mappings"][number];

/**
 * One row of `/hevy/mappings`. Unlike `HevyMappingSummary`'s row — which only
 * ever renders exercises still needing action — this covers the mapped case
 * too: it names the current Garmin and Strava destinations and offers Edit.
 */
export function MappingListRow({ mapping }: { mapping: MappingRowData }) {
  const verb = mapping.mapped ? "Edit" : "Map";
  // Both names come from the same taxonomy lookup, so they are present or
  // absent together — but guard on both rather than render a bare "›".
  const garmin =
    mapping.categoryName && mapping.subcategoryName
      ? `${mapping.categoryName} › ${mapping.subcategoryName}`
      : "Mapped";

  return (
    <li className="flex items-center gap-3.5 border-b border-border/50 px-5 py-3.5 last:border-b-0">
      <div className="min-w-0 flex-1">
        <p className="truncate text-[14px] font-semibold">
          {mapping.title}
          {mapping.isCustom ? <Tag>CUSTOM</Tag> : null}
        </p>
        <dl className="mt-1 grid grid-cols-[4rem_minmax(0,1fr)] gap-x-2 gap-y-0.5 font-mono text-xs">
          <Destination
            label="Garmin"
            value={mapping.unmapped ? null : garmin}
            empty="Unmapped"
            // Most exercises are resolved by the ported tables, not chosen
            // by anyone — without this a standard pair reads as a
            // deliberate decision the user made and forgot.
            override={mapping.source === "user"}
          />
          <Destination
            label="Strava"
            value={mapping.stravaExerciseName}
            empty="Not set"
            override={mapping.stravaSource === "user"}
          />
        </dl>
      </div>
      <Button
        asChild
        variant={mapping.unmapped ? "warning" : "outline"}
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

function Destination({
  label,
  value,
  empty,
  override,
}: {
  label: string;
  value: string | null;
  empty: string;
  override: boolean;
}) {
  return (
    <>
      <dt className="text-[10px] leading-[18px] tracking-[0.08em] text-muted-foreground/60 uppercase">
        {label}
      </dt>
      <dd className="min-w-0 truncate text-muted-foreground">
        {value ?? empty}
        {override ? <Tag>Your override</Tag> : null}
      </dd>
    </>
  );
}

function Tag({ children }: { children: string }) {
  return (
    <span className="ml-1.5 rounded bg-secondary px-1.5 py-0.5 align-middle font-mono text-[10px] text-muted-foreground">
      {children}
    </span>
  );
}
