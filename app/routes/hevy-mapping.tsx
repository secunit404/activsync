import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useOutletContext, useParams } from "react-router";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Field, FieldLabel } from "@/components/ui/field";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { ResponsiveOverlay } from "@/components/ui/responsive-overlay";
import { Spinner } from "@/components/ui/spinner";
import { getHevyTools, saveExerciseMapping, type HevyToolsState } from "@/lib/api";
import type { BackfillOutletContext } from "./hevy-backfill";
import { queryKeys } from "@/lib/query-keys";
import { ERROR_TOAST_DURATION_MS } from "@/lib/toast-duration";

type Mapping = HevyToolsState["mappings"][number];
type Category = HevyToolsState["categories"][number];

/**
 * Exercise mapping editor (frame `4a`): the exceptions workflow for Hevy
 * exercises Garmin can't resolve on its own. Garmin applies a default
 * mapping automatically the moment Hevy connects (`hevy_mapper.py`'s
 * suggestion pass, run by `view.hevy_mappings_view`) — this overlay only
 * ever opens for the leftovers: exercises with no default, and ones Garmin
 * rejected after a prior save.
 *
 * Nested under `/hevy` (Task 14's hub, whose own `<Outlet />` mounts this
 * route with no context — see that route's docstring), so this issues its
 * own `getHevyTools` query against the same `queryKeys.hevyTools` key the
 * hub already fetched. The hub gates its `<Outlet />` on that query
 * resolving, so by the time this route can mount the data is already
 * cached — the `isPending`/`isError` branches below exist for completeness
 * (a hot reload, a future consumer that doesn't gate the same way) rather
 * than a path normal navigation exercises.
 */
export default function HevyMapping() {
  const { templateId } = useParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  // `useOutletContext` returns undefined when this route is mounted directly
  // under `/hevy` (the hub renders `<Outlet />` with no context), so this is
  // optional by construction, not by defensive coding.
  const outletContext = useOutletContext<BackfillOutletContext | undefined>();
  const tools = useQuery({
    queryKey: queryKeys.hevyTools,
    queryFn: ({ signal }) => getHevyTools(signal),
  });

  // Where Cancel/Save return to. This module is mounted at two paths — under
  // `/hevy` (from the hub) and under `/hevy/backfill` (from a locked row) —
  // and ".." resolves correctly for both: the parent route is exactly the
  // screen the user came from. That replaces the old `location.key ===
  // "default"` heuristic, which had to guess whether there was a history
  // entry to go back to.
  const close = () => navigate("..");
  const handleOpenChange = (open: boolean) => {
    if (!open) {
      close();
    }
  };

  const save = useMutation({
    mutationFn: ({ category, subcategory }: { category: number; subcategory: number }) =>
      saveExerciseMapping(templateId ?? "", category, subcategory),
    onSuccess: (result) => {
      toast.success(result.message);
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyTools });
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue });
      // Re-run the parent's preview when nested under backfill, so the row
      // just mapped unlocks in place instead of showing stale scan data.
      outletContext?.onMappingSaved();
      close();
    },
    onError: (error) => {
      toast.error(error instanceof Error ? error.message : "Could not save exercise mapping.", {
        duration: ERROR_TOAST_DURATION_MS,
      });
    },
  });

  if (tools.isPending) {
    return (
      <ResponsiveOverlay open onOpenChange={handleOpenChange} title="Loading…" mobile="sheet">
        <div className="grid place-items-center py-10 text-muted-foreground">
          <Spinner className="size-5" aria-label="Loading exercise mapping" />
        </div>
      </ResponsiveOverlay>
    );
  }

  if (tools.isError) {
    return (
      <ResponsiveOverlay
        open
        onOpenChange={handleOpenChange}
        title="Couldn't load Garmin categories"
        mobile="sheet"
      >
        <p className="text-sm text-muted-foreground">
          {tools.error instanceof Error
            ? tools.error.message
            : "Something went wrong loading the Garmin taxonomy. Close this and try again."}
        </p>
      </ResponsiveOverlay>
    );
  }

  const mapping = tools.data.mappings.find((row) => row.templateId === templateId);

  if (!mapping) {
    // A stale/shared link, or a mapping just saved/removed elsewhere — the
    // template id no longer resolves against the current list. Render a
    // real explanatory overlay, per the brief's explicit "must not render a
    // blank overlay" (same pattern as activity-detail's not-found case).
    return (
      <ResponsiveOverlay open onOpenChange={handleOpenChange} title="Exercise not found" mobile="sheet">
        <p className="text-sm text-muted-foreground">
          This exercise isn&apos;t in the current mapping list — it may already be mapped, or the
          list has changed. Close this and check the Hevy hub.
        </p>
      </ResponsiveOverlay>
    );
  }

  return (
    <ExerciseMappingForm
      mapping={mapping}
      categories={tools.data.categories}
      saving={save.isPending}
      onCancel={close}
      onOpenChange={handleOpenChange}
      onSave={(category, subcategory) => save.mutate({ category, subcategory })}
    />
  );
}

function ExerciseMappingForm({
  mapping,
  categories,
  saving,
  onCancel,
  onOpenChange,
  onSave,
}: {
  mapping: Mapping;
  categories: Category[];
  saving: boolean;
  onCancel: () => void;
  onOpenChange: (open: boolean) => void;
  onSave: (category: number, subcategory: number) => void;
}) {
  const [category, setCategory] = useState(
    mapping.category !== null ? String(mapping.category) : "",
  );
  const [subcategory, setSubcategory] = useState(
    mapping.subcategory !== null ? String(mapping.subcategory) : "",
  );

  // Re-seed the draft whenever the editor lands on a different exercise —
  // render-time state adjustment (not a `useEffect`; this repo's ESLint
  // enforces `react-hooks/set-state-in-effect`), same technique as
  // activity-detail's `fieldsResetKey`. A `Map →` click from one exercise
  // straight to another reuses this component instance (same route, new
  // `:templateId`), so without this the previous exercise's draft would
  // leak into the next one's selects.
  const [appliedTemplateId, setAppliedTemplateId] = useState(mapping.templateId);
  if (appliedTemplateId !== mapping.templateId) {
    setAppliedTemplateId(mapping.templateId);
    setCategory(mapping.category !== null ? String(mapping.category) : "");
    setSubcategory(mapping.subcategory !== null ? String(mapping.subcategory) : "");
  }

  const selectedCategory = categories.find((option) => String(option.value) === category);
  const selectedSubcategory = selectedCategory?.subcategories.find(
    (option) => String(option.value) === subcategory,
  );

  // Some Garmin categories (cycling, yoga, treadmill, …) carry zero
  // subcategories. Offering them is a dead end: the subcategory select would
  // enable with nothing but the placeholder in it, and `canSave` below can
  // never turn true because a subcategory can never be chosen — `POST
  // /mappings/{id}` independently rejects every subcategory for such a
  // category too, since `SUBCATEGORY_NAMES.get(category, {})` is `{}`
  // (`hevy_tools_api_routes.py`). Derived from the data, not a hardcoded
  // id list, since the taxonomy comes from Garmin and can change.
  //
  // Exception: if the exercise's already-saved mapping points at one of
  // these categories, keep it selectable instead of silently dropping it
  // from the list. `save_mapping` can never *write* such a pair today (see
  // above), so this is unreachable through normal use — but it means a
  // saved value never vanishes out from under the picker if the taxonomy
  // ever changes shape later.
  const selectableCategories = categories.filter(
    (option) => option.subcategories.length > 0 || option.value === selectedCategory?.value,
  );

  // Subcategories belong to their parent category — switching category
  // must drop whatever subcategory was picked under the old one, or Save
  // could write a (category, subcategory) pair that never appeared
  // together in the dropdown. Both fields update together in this one
  // event handler, not an effect, so there's no intermediate render where
  // they briefly disagree.
  function handleCategoryChange(value: string) {
    setCategory(value);
    setSubcategory("");
  }

  const canSave = category !== "" && subcategory !== "";

  return (
    <ResponsiveOverlay
      open
      onOpenChange={onOpenChange}
      title={mapping.title}
      description={`${mapping.muscleGroup ? `${mapping.muscleGroup} · ` : ""}Garmin applies a default mapping automatically when Hevy connects — this editor is only for the exercises it couldn't resolve on its own.`}
      mobile="sheet"
      footer={
        <div className="flex gap-2.5">
          <Button
            type="button"
            variant="outline"
            className="flex-1 md:flex-none"
            disabled={saving}
            onClick={onCancel}
          >
            Cancel
          </Button>
          <Button
            type="button"
            className="flex-1 md:flex-none"
            disabled={!canSave || saving}
            onClick={() => onSave(Number(category), Number(subcategory))}
          >
            {saving ? <Spinner data-icon="inline-start" /> : null}
            {saving ? "Saving…" : "Save mapping"}
          </Button>
        </div>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="flex items-start gap-2.5 rounded-[10px] border border-warning/22 bg-warning/[0.06] px-3.5 py-2.5">
          <span className="mt-[5px] size-1.5 shrink-0 rounded-full bg-warning" aria-hidden="true" />
          <p className="text-[12.5px] leading-relaxed text-warning/90">
            {mapping.garminRejected
              ? "Garmin rejected the previous mapping — pick a different category and subcategory."
              : "No Garmin default exists for this exercise. Pick where its sets and reps should be recorded."}
          </p>
        </div>

        <Field>
          <FieldLabel
            htmlFor="mapping-category"
            className="font-mono text-xs tracking-[0.06em] text-muted-foreground uppercase"
          >
            Category
          </FieldLabel>
          <NativeSelect
            id="mapping-category"
            className="w-full"
            value={category}
            onChange={(event) => handleCategoryChange(event.target.value)}
          >
            <NativeSelectOption value="">Select a category…</NativeSelectOption>
            {selectableCategories.map((option) => (
              <NativeSelectOption key={option.value} value={String(option.value)}>
                {option.label}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </Field>

        <Field>
          <FieldLabel
            htmlFor="mapping-subcategory"
            className="font-mono text-xs tracking-[0.06em] text-muted-foreground uppercase"
          >
            Subcategory
          </FieldLabel>
          <NativeSelect
            id="mapping-subcategory"
            className="w-full"
            value={subcategory}
            disabled={!selectedCategory}
            onChange={(event) => setSubcategory(event.target.value)}
          >
            <NativeSelectOption value="">
              {selectedCategory ? "Select a subcategory…" : "Choose a category first"}
            </NativeSelectOption>
            {(selectedCategory?.subcategories ?? []).map((option) => (
              <NativeSelectOption key={option.value} value={String(option.value)}>
                {option.label}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </Field>

        <div className="flex items-center gap-2.5 rounded-[10px] border border-border bg-muted/20 px-3.5 py-2.5">
          <span className="font-mono text-xs text-muted-foreground uppercase">Syncs as</span>
          <span data-testid="syncs-as" className="font-mono text-[13.5px] font-semibold text-success">
            {selectedCategory && selectedSubcategory
              ? `${selectedCategory.label} › ${selectedSubcategory.label}`
              : "Pick both fields to see the result"}
          </span>
        </div>
      </div>
    </ResponsiveOverlay>
  );
}
