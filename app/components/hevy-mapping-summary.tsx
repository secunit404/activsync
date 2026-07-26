import { Link } from "react-router";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { HevyToolsState } from "@/lib/api";

type MappingRowData = HevyToolsState["mappings"][number];

/**
 * Frame `3a`'s mapping-attention card, condensed to a summary per the Task
 * 14 brief: a needs-mapping count with inline links into the mapping editor
 * (`/hevy/mapping/:templateId`, built in Task 15). `getHevyTools` already
 * only returns rows a user can act on — built-in exercises Garmin resolves
 * on its own never appear (see `view.hevy_mappings_view`) — so this only has
 * to split those rows into "needs a mapping" versus "already configured".
 */
export function HevyMappingSummary({ tools }: { tools: HevyToolsState }) {
  const needsAction = tools.mappings.filter((mapping) => mapping.unmapped);

  return (
    <Card id="hevy-mapping" className="gap-0 py-0" aria-labelledby="hevy-mapping-title">
      <CardHeader className="border-b border-border/70 py-4">
        <CardTitle>
          <h2 id="hevy-mapping-title" className="text-[15px] font-bold">
            Exercises needing mapping
          </h2>
        </CardTitle>
      </CardHeader>
      <CardContent className="p-0">
        {needsAction.length === 0 ? (
          <p className="px-5 py-6 text-sm text-muted-foreground">
            Every Hevy exercise Garmin can't resolve on its own has a mapping.
          </p>
        ) : (
          <ul>
            {needsAction.map((mapping) => (
              <MappingRow key={mapping.templateId} mapping={mapping} />
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function MappingRow({ mapping }: { mapping: MappingRowData }) {
  const verb = "Map";
  return (
    <li className="flex items-center gap-3.5 border-b border-border/50 px-5 py-3.5 last:border-b-0">
      <div className="min-w-0 flex-1">
        <p className="truncate text-[14px] font-semibold">
          {mapping.title}
          {mapping.isCustom ? (
            <span className="ml-1.5 rounded bg-secondary px-1.5 py-0.5 align-middle font-mono text-[10px] text-muted-foreground">
              CUSTOM
            </span>
          ) : null}
        </p>
        <p className="truncate text-xs text-muted-foreground">Unmapped</p>
      </div>
      <Button
        asChild
        variant="warning"
        size="sm"
        className="shrink-0"
      >
        <Link
          to={`/hevy/mapping/${encodeURIComponent(mapping.templateId)}`}
          aria-label={`${verb} ${mapping.title}`}
        >
          {verb} →
        </Link>
      </Button>
    </li>
  );
}
