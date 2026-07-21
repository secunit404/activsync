import { useMutation, useQuery } from "@tanstack/react-query";
import { DumbbellIcon } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  NativeSelect,
  NativeSelectOption,
} from "@/components/ui/native-select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { useSettingsAction } from "@/hooks/use-settings-action";
import {
  getHevyTools,
  previewHevyBackfill,
  removeExerciseMapping,
  runHevyBackfill,
  saveExerciseMapping,
  type BackfillResult,
  type HevyToolsState,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

export function HevyTools() {
  const tools = useQuery({
    queryKey: queryKeys.hevyTools,
    queryFn: ({ signal }) => getHevyTools(signal),
  });

  if (tools.isPending) {
    return (
      <div className="grid gap-3 border-t pt-6">
        <Skeleton className="h-6 w-40" />
        <Skeleton className="h-24 w-full" />
      </div>
    );
  }
  if (tools.isError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Couldn’t load Hevy tools</AlertTitle>
        <AlertDescription>{tools.error.message}</AlertDescription>
      </Alert>
    );
  }
  return (
    <div className="grid gap-8 border-t pt-6">
      <BackfillTools />
      <MappingTools state={tools.data} />
    </div>
  );
}

function MappingTools({ state }: { state: HevyToolsState }) {
  return (
    <section
      id="hevy-mappings"
      className="grid gap-4"
      aria-labelledby="exercise-mappings-title"
    >
      <div className="grid gap-1">
        <h3 id="exercise-mappings-title" className="text-lg font-semibold">
          Exercise mappings
        </h3>
        <p className="text-sm text-muted-foreground">
          Confirm how custom or unresolved Hevy exercises are represented in
          Garmin.
        </p>
      </div>
      {state.mappings.length ? (
        <div className="grid gap-3">
          {state.mappings.map((mapping) => (
            <MappingEditor
              key={mapping.templateId}
              mapping={mapping}
              categories={state.categories}
            />
          ))}
        </div>
      ) : (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <DumbbellIcon />
            </EmptyMedia>
            <EmptyTitle>Nothing to map yet</EmptyTitle>
            <EmptyDescription>
              Custom exercises appear here after Hevy templates are fetched.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
    </section>
  );
}

function MappingEditor({
  mapping,
  categories,
}: {
  mapping: HevyToolsState["mappings"][number];
  categories: HevyToolsState["categories"];
}) {
  const fallbackCategory = categories[0]?.value ?? 0;
  const [category, setCategory] = useState(
    mapping.category ?? fallbackCategory,
  );
  const activeCategory =
    categories.find((item) => item.value === category) ?? categories[0];
  const fallbackSubcategory = activeCategory?.subcategories[0]?.value ?? 0;
  const [subcategory, setSubcategory] = useState(
    mapping.subcategory ?? fallbackSubcategory,
  );
  const save = useSettingsAction(
    (payload: { category: number; subcategory: number }) =>
      saveExerciseMapping(mapping.templateId, payload.category, payload.subcategory),
  );
  const remove = useSettingsAction(() =>
    removeExerciseMapping(mapping.templateId),
  );

  return (
    <article className="grid gap-4 rounded-xl border bg-muted/15 p-4">
      <header className="flex flex-wrap items-start gap-2">
        <div className="mr-auto min-w-0">
          <h4 className="font-semibold">{mapping.title}</h4>
          {mapping.muscleGroup ? (
            <p className="text-sm text-muted-foreground">
              {mapping.muscleGroup}
            </p>
          ) : null}
        </div>
        {mapping.isCustom ? <Badge variant="outline">Custom</Badge> : null}
        <MappingStatus mapping={mapping} />
      </header>
      <form
        className="grid gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          save.mutate({ category, subcategory });
        }}
      >
        <FieldGroup className="grid gap-4 sm:grid-cols-2">
          <Field>
            <FieldLabel htmlFor={"mapping-category-" + mapping.templateId}>
              Garmin category
            </FieldLabel>
            <NativeSelect
              id={"mapping-category-" + mapping.templateId}
              className="w-full"
              value={category}
              onChange={(event) => {
                const nextCategory = Number(event.target.value);
                const nextOptions = categories.find(
                  (item) => item.value === nextCategory,
                )?.subcategories;
                setCategory(nextCategory);
                setSubcategory(nextOptions?.[0]?.value ?? 0);
              }}
            >
              {categories.map((item) => (
                <NativeSelectOption key={item.value} value={item.value}>
                  {item.label}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
          <Field>
            <FieldLabel htmlFor={"mapping-subcategory-" + mapping.templateId}>
              Garmin exercise
            </FieldLabel>
            <NativeSelect
              id={"mapping-subcategory-" + mapping.templateId}
              className="w-full"
              value={subcategory}
              onChange={(event) => setSubcategory(Number(event.target.value))}
            >
              {(activeCategory?.subcategories ?? []).map((item) => (
                <NativeSelectOption key={item.value} value={item.value}>
                  {item.label}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
        </FieldGroup>
        <div className="flex flex-col gap-2 sm:flex-row">
          <Button
            type="submit"
            className="h-11"
            disabled={save.isPending || !activeCategory}
          >
            {save.isPending ? <Spinner /> : null}
            {save.isPending ? "Saving…" : "Save mapping"}
          </Button>
          {mapping.mapped ? (
            <Button
              type="button"
              variant="outline"
              className="h-11"
              disabled={remove.isPending}
              onClick={() => remove.mutate()}
            >
              {remove.isPending ? <Spinner /> : null}
              {remove.isPending ? "Removing…" : "Remove mapping"}
            </Button>
          ) : null}
        </div>
      </form>
    </article>
  );
}

function MappingStatus({
  mapping,
}: {
  mapping: HevyToolsState["mappings"][number];
}) {
  if (mapping.garminRejected) {
    return <Badge variant="destructive">Rejected by Garmin</Badge>;
  }
  if (mapping.mapped) {
    return <Badge variant="secondary">Mapped</Badge>;
  }
  if (mapping.unmapped) {
    return <Badge variant="outline">Needs mapping</Badge>;
  }
  return <Badge variant="ghost">Suggested</Badge>;
}

function BackfillTools() {
  const [since, setSince] = useState("");
  const [plan, setPlan] = useState<BackfillResult | null>(null);
  const preview = useMutation({
    mutationFn: previewHevyBackfill,
    onSuccess: (result) => {
      setPlan(result);
      toast.info(result.message);
    },
    onError: (error) => toast.error(error.message),
  });
  const run = useSettingsAction(runHevyBackfill);
  const pending = preview.isPending || run.isPending;

  return (
    <section className="grid gap-4" aria-labelledby="hevy-backfill-title">
      <div className="grid gap-1">
        <h3 id="hevy-backfill-title" className="text-lg font-semibold">
          Backfill history
        </h3>
        <p className="text-sm text-muted-foreground">
          Preview older workouts before writing anything. Exact Garmin twins are
          linked, never uploaded again.
        </p>
      </div>
      <form
        className="grid gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          preview.mutate(since);
        }}
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="hevy-backfill-since">
              Backfill workouts since
            </FieldLabel>
            <Input
              id="hevy-backfill-since"
              className="h-11 sm:max-w-xs"
              type="date"
              value={since}
              onChange={(event) => {
                setSince(event.target.value);
                setPlan(null);
              }}
              required
            />
            <FieldDescription>
              Preview is read-only. The run button appears after review.
            </FieldDescription>
          </Field>
        </FieldGroup>
        <Button
          type="submit"
          variant="outline"
          className="h-11 sm:w-fit"
          disabled={pending}
        >
          {preview.isPending ? <Spinner /> : null}
          {preview.isPending ? "Fetching…" : "Preview backfill"}
        </Button>
      </form>
      {plan ? (
        <BackfillPlan
          plan={plan}
          running={run.isPending}
          onRun={async () => {
            if (
              !window.confirm(
                "Run this backfill? Listed twins are linked; the rest are ingested for the next sync tick.",
              )
            ) {
              return;
            }
            const result = await run.mutateAsync(plan.since);
            setPlan(result);
          }}
        />
      ) : null}
    </section>
  );
}

function BackfillPlan({
  plan,
  running,
  onRun,
}: {
  plan: BackfillResult;
  running: boolean;
  onRun: () => Promise<void>;
}) {
  return (
    <div className="grid gap-4 rounded-xl border bg-muted/15 p-4">
      <Alert>
        <AlertTitle>{plan.ran ? "Backfill complete" : "Preview ready"}</AlertTitle>
        <AlertDescription>{plan.message}</AlertDescription>
      </Alert>
      {plan.items.length ? (
        <ul className="grid gap-2">
          {plan.items.map((item) => (
            <li
              key={item.hevyId}
              className="grid gap-1 rounded-lg border bg-background px-3 py-2 sm:grid-cols-[1fr_auto] sm:items-center"
            >
              <div className="min-w-0">
                <p className="truncate font-medium">{item.title}</p>
                <p className="text-xs text-muted-foreground">
                  {formatWorkoutTime(item.startTime)}
                </p>
              </div>
              <Badge variant="outline">{backfillAction(item)}</Badge>
            </li>
          ))}
        </ul>
      ) : null}
      {!plan.ran && plan.items.length ? (
        <Button
          className="h-11 sm:w-fit"
          disabled={running}
          onClick={() => void onRun()}
        >
          {running ? <Spinner /> : null}
          {running ? "Running…" : "Run backfill"}
        </Button>
      ) : null}
    </div>
  );
}

function backfillAction(item: BackfillResult["items"][number]) {
  if (item.action === "linked_existing" && item.twinActivityId !== null) {
    return "Link activity " + item.twinActivityId;
  }
  return item.action.replaceAll("_", " ");
}

function formatWorkoutTime(value: string) {
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf()) ? value : parsed.toLocaleString();
}
