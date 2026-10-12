import { useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useOutletContext, useParams } from "react-router";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import {
  NativeSelect,
  NativeSelectOptGroup,
  NativeSelectOption,
} from "@/components/ui/native-select";
import { ResponsiveOverlay } from "@/components/ui/responsive-overlay";
import { Spinner } from "@/components/ui/spinner";
import {
  getHevyTools,
  removeExerciseMapping,
  removeStravaExerciseMapping,
  saveExerciseMapping,
  saveStravaExerciseMapping,
  type HevyToolsState,
} from "@/lib/api";
import type { HevyOutletContext } from "./hevy";
import { queryKeys } from "@/lib/query-keys";
import { ERROR_TOAST_DURATION_MS } from "@/lib/toast-duration";

type Mapping = HevyToolsState["mappings"][number];
type Category = HevyToolsState["categories"][number];
type StravaGroup = HevyToolsState["stravaGroups"][number];

type SaveRequest = {
  category: number;
  subcategory: number;
  saveGarmin: boolean;
  stravaExerciseType: string | null;
};

/**
 * Exercise mapping editor. Opens for any exercise, not just the unresolved
 * ones — most are placed by the ported tables (see
 * `view.hevy_mappings_view`), and this is where that choice can be inspected
 * and overridden. The banner below names which of the three states applies.
 *
 * Mounted in `/hevy`'s `<Outlet />` as an overlay over the hub, so it issues
 * its own `getHevyTools` query against the same `queryKeys.hevyTools` key the
 * hub already fetched. The hub gates its `<Outlet />` on that query
 * resolving, so by the time this route can mount the data is already cached —
 * the `isPending`/`isError` branches below exist for completeness (a hot
 * reload, a future consumer that doesn't gate the same way) rather than a
 * path normal navigation exercises.
 */
export default function HevyMapping() {
  const { templateId } = useParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  // Optional so this component stays renderable in tests without the hub as
  // an ancestor; in the app the hub always supplies it.
  const outletContext = useOutletContext<HevyOutletContext | undefined>();
  const tools = useQuery({
    queryKey: queryKeys.hevyTools,
    queryFn: ({ signal }) => getHevyTools(signal),
  });

  // Structural rather than inferred: the parent route IS the hub, which is
  // the only screen this can be opened from now that backfill is inline
  // there. Replaces an older `location.key === "default"` heuristic that had
  // to guess whether there was a history entry worth going back to.
  const close = () => navigate("..");
  const handleOpenChange = (open: boolean) => {
    if (!open) {
      close();
    }
  };

  const save = useMutation({
    mutationFn: async ({ category, subcategory, saveGarmin, stravaExerciseType }: SaveRequest) => {
      const id = templateId ?? "";
      const garmin = saveGarmin ? await saveExerciseMapping(id, category, subcategory) : null;
      const strava = stravaExerciseType
        ? await saveStravaExerciseMapping(id, stravaExerciseType)
        : null;
      return garmin ?? strava ?? { message: "Nothing to save." };
    },
    onSuccess: (result) => {
      toast.success(result.message);
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyTools });
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue });
      // Re-scan the hub's inline backfill card, so a row just mapped unlocks
      // in place instead of keeping the previous scan's verdict.
      outletContext?.onMappingSaved();
      close();
    },
    onError: (error) => {
      toast.error(error instanceof Error ? error.message : "Could not save exercise mapping.", {
        duration: ERROR_TOAST_DURATION_MS,
      });
    },
  });

  const reset = useMutation({
    mutationFn: async ({ garmin, strava }: { garmin: boolean; strava: boolean }) => {
      const id = templateId ?? "";
      const garminResult = garmin ? await removeExerciseMapping(id) : null;
      const stravaResult = strava ? await removeStravaExerciseMapping(id) : null;
      return garminResult ?? stravaResult ?? { message: "Nothing to reset." };
    },
    onSuccess: (result) => {
      toast.success(result.message);
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyTools });
      void queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue });
      outletContext?.onMappingSaved();
      close();
    },
    onError: (error) => {
      toast.error(error instanceof Error ? error.message : "Could not reset exercise mapping.", {
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
      stravaGroups={tools.data.stravaGroups}
      saving={save.isPending}
      resetting={reset.isPending}
      onCancel={close}
      onOpenChange={handleOpenChange}
      onSave={(request) => save.mutate(request)}
      onReset={(scope) => reset.mutate(scope)}
    />
  );
}

function ExerciseMappingForm({
  mapping,
  categories,
  stravaGroups,
  saving,
  resetting,
  onCancel,
  onOpenChange,
  onSave,
  onReset,
}: {
  mapping: Mapping;
  categories: Category[];
  stravaGroups: StravaGroup[];
  saving: boolean;
  resetting: boolean;
  onCancel: () => void;
  onOpenChange: (open: boolean) => void;
  onSave: (request: SaveRequest) => void;
  onReset: (scope: { garmin: boolean; strava: boolean }) => void;
}) {
  const [category, setCategory] = useState(
    mapping.category !== null ? String(mapping.category) : "",
  );
  const [subcategory, setSubcategory] = useState(
    mapping.subcategory !== null ? String(mapping.subcategory) : "",
  );
  const [stravaExercise, setStravaExercise] = useState(mapping.stravaExerciseType ?? "");

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
    setStravaExercise(mapping.stravaExerciseType ?? "");
  }

  const selectedCategory = categories.find((option) => String(option.value) === category);

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
  const hasPersistedOverride = mapping.source === "user";
  const standardCategory =
    mapping.standardCategory !== null ? String(mapping.standardCategory) : "";
  const standardSubcategory =
    mapping.standardSubcategory !== null ? String(mapping.standardSubcategory) : "";
  const draftDiffersFromStandard =
    mapping.hasStandardMapping &&
    (category !== standardCategory || subcategory !== standardSubcategory);
  const garminCanReset =
    mapping.hasStandardMapping && (hasPersistedOverride || draftDiffersFromStandard);
  const stravaStandard = mapping.stravaStandardExerciseType ?? "";
  const hasPersistedStravaOverride = mapping.stravaSource === "user";
  const stravaCanReset =
    stravaStandard !== "" &&
    (hasPersistedStravaOverride || stravaExercise !== stravaStandard);
  const canReset = garminCanReset || stravaCanReset;
  const stravaChanged = stravaExercise !== (mapping.stravaExerciseType ?? "");
  const busy = saving || resetting;

  function handleReset() {
    const scope = {
      garmin: garminCanReset && hasPersistedOverride,
      strava: stravaCanReset && hasPersistedStravaOverride,
    };
    if (scope.garmin || scope.strava) {
      onReset(scope);
      return;
    }
    if (garminCanReset) {
      setCategory(standardCategory);
      setSubcategory(standardSubcategory);
    }
    if (stravaCanReset) {
      setStravaExercise(stravaStandard);
    }
  }

  function handleSave() {
    // Saving only a new Strava exercise must not turn the standard Garmin
    // pair into a user override it was never meant to be.
    const garminUnchanged =
      category === String(mapping.category) && subcategory === String(mapping.subcategory);
    onSave({
      category: Number(category),
      subcategory: Number(subcategory),
      saveGarmin: !(mapping.source === "automatic" && garminUnchanged && stravaChanged),
      stravaExerciseType: stravaChanged && stravaExercise !== "" ? stravaExercise : null,
    });
  }

  return (
    <ResponsiveOverlay
      open
      onOpenChange={onOpenChange}
      title={mapping.title}
      description={
        mapping.muscleGroup ? (
          <span>
            <span className="font-medium text-foreground">Muscle group:</span>{" "}
            {formatMuscleGroup(mapping.muscleGroup)}
          </span>
        ) : undefined
      }
      mobile="sheet"
      footer={
        <div
          data-testid="mapping-actions"
          className="flex flex-wrap gap-2.5 md:justify-end"
        >
          {canReset ? (
            <Button
              type="button"
              variant="ghost"
              className="w-full text-muted-foreground md:mr-auto md:w-auto"
              disabled={busy}
              onClick={handleReset}
            >
              {resetting ? <Spinner data-icon="inline-start" /> : null}
              {resetting ? "Resetting…" : "Reset to standard"}
            </Button>
          ) : null}
          <Button
            type="button"
            variant="outline"
            className="flex-1 md:flex-none"
            disabled={busy}
            onClick={onCancel}
          >
            Cancel
          </Button>
          <Button
            type="button"
            className="flex-1 md:flex-none"
            disabled={!canSave || busy}
            onClick={handleSave}
          >
            {saving ? <Spinner data-icon="inline-start" /> : null}
            {saving ? "Saving…" : "Save mapping"}
          </Button>
        </div>
      }
    >
      <div className="flex flex-col gap-4">
        {mapping.unmapped && mapping.suggested ? (
          <div className="rounded-lg border border-warning/30 bg-warning/[0.06] p-3.5 text-sm">
            <p className="font-semibold text-warning">Suggested mapping (not active yet)</p>
            <p className="mt-1 leading-relaxed text-muted-foreground">
              ActivSync preselected a best guess from the closest exercise name or muscle group.
              Review it and press Save mapping before structured sets can sync.
            </p>
          </div>
        ) : null}
        <DestinationSection
          title="Garmin"
          description="Where this exercise's sets land on your Garmin activity."
          status={
            mapping.unmapped
              ? { label: "Not mapped", tone: "warning" }
              : mapping.source === "user"
                ? { label: "Your override", tone: "muted" }
                : { label: "Standard", tone: "muted" }
          }
        >
          <Field>
            <FieldLabel htmlFor="mapping-category" className={FIELD_LABEL}>
              Category
            </FieldLabel>
            <NativeSelect
              id="mapping-category"
              className="w-full"
              value={category}
              disabled={busy}
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
            <FieldLabel htmlFor="mapping-subcategory" className={FIELD_LABEL}>
              Exercise
            </FieldLabel>
            <NativeSelect
              id="mapping-subcategory"
              aria-label="Garmin exercise"
              className="w-full"
              value={subcategory}
              disabled={!selectedCategory || busy}
              onChange={(event) => setSubcategory(event.target.value)}
            >
              <NativeSelectOption value="">
                {selectedCategory ? "Select an exercise…" : "Choose a category first"}
              </NativeSelectOption>
              {(selectedCategory?.subcategories ?? []).map((option) => (
                <NativeSelectOption key={option.value} value={String(option.value)}>
                  {option.label}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
        </DestinationSection>

        <DestinationSection
          title="Strava"
          description="The exercise Strava shows when a Hevy workout is uploaded."
          status={STRAVA_STATUS[mapping.stravaSource]}
        >
          <Field>
            <FieldLabel htmlFor="mapping-strava-exercise" className={FIELD_LABEL}>
              Exercise
            </FieldLabel>
            <NativeSelect
              id="mapping-strava-exercise"
              aria-label="Strava exercise"
              className="w-full"
              value={stravaExercise}
              disabled={busy}
              onChange={(event) => setStravaExercise(event.target.value)}
            >
              <NativeSelectOption value="">
                {mapping.stravaExerciseType
                  ? "Select a Strava exercise…"
                  : "Not set, so the workout uploads as a FIT file"}
              </NativeSelectOption>
              {stravaGroups.map((group) => (
                <NativeSelectOptGroup key={group.label} label={group.label}>
                  {group.exercises.map((option) => (
                    <NativeSelectOption key={option.value} value={option.value}>
                      {option.label}
                    </NativeSelectOption>
                  ))}
                </NativeSelectOptGroup>
              ))}
            </NativeSelect>
            {mapping.stravaSource === "garmin" && stravaExercise === mapping.stravaExerciseType ? (
              <FieldDescription>
                No standard Strava exercise, so this follows the Garmin exercise above. Pick one
                to override.
              </FieldDescription>
            ) : null}
          </Field>
        </DestinationSection>
      </div>
    </ResponsiveOverlay>
  );
}

const FIELD_LABEL = "font-mono text-xs tracking-[0.06em] text-muted-foreground uppercase";

type SectionStatus = { label: string; tone: "muted" | "warning" };

const STRAVA_STATUS: Record<Mapping["stravaSource"], SectionStatus> = {
  user: { label: "Your override", tone: "muted" },
  standard: { label: "Standard", tone: "muted" },
  garmin: { label: "Matched to Garmin", tone: "muted" },
  "": { label: "Not set", tone: "warning" },
};

function DestinationSection({
  title,
  description,
  status,
  children,
}: {
  title: string;
  description: string;
  status: SectionStatus;
  children: ReactNode;
}) {
  return (
    <section
      aria-label={title}
      className="flex flex-col gap-4 rounded-xl border border-border bg-foreground/[0.02] p-4"
    >
      <header className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold">{title}</h3>
          <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{description}</p>
        </div>
        <span
          className={
            status.tone === "warning"
              ? "shrink-0 rounded bg-warning/10 px-1.5 py-0.5 font-mono text-[10px] text-warning"
              : "shrink-0 rounded bg-secondary px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
          }
        >
          {status.label}
        </span>
      </header>
      {children}
    </section>
  );
}

function formatMuscleGroup(value: string): string {
  const label = value.replaceAll("_", " ").toLowerCase();
  return label.slice(0, 1).toUpperCase() + label.slice(1);
}
