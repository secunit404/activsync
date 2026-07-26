import { ChevronDownIcon } from "lucide-react";

import type { HevyWorkoutDetail, HevyWorkoutSetDetail } from "@/lib/api";

export function HevyWorkoutDetailContent({ detail }: { detail: HevyWorkoutDetail }) {
  return (
    <div className="space-y-6">
      {/* No fill. This is a two-value readout, not a panel or a well — the
          border alone groups it, and any fill made it compete with the
          exercise list below for attention it doesn't need. */}
      <div className="grid grid-cols-2 gap-3 rounded-lg border border-border/70 p-4 text-sm">
        <DetailValue label="Started" value={formatWorkoutTime(detail.startTime)} />
        <DetailValue label="Duration" value={formatDuration(detail.startTime, detail.endTime)} />
      </div>

      {detail.notes ? (
        <section>
          <h3 className="mb-2 text-sm font-bold">Workout notes</h3>
          <p className="whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground">
            {detail.notes}
          </p>
        </section>
      ) : null}

      <section>
        <h3 className="mb-3 text-sm font-bold">Exercises · {detail.exercises.length}</h3>
        {detail.exercises.length === 0 ? (
          <p className="text-sm text-muted-foreground">No exercises were recorded.</p>
        ) : (
          <ol className="space-y-3">
            {detail.exercises.map((exercise, exerciseIndex) => (
              <li
                key={`${exercise.templateId ?? exercise.title}-${exerciseIndex}`}
                className="rounded-lg border border-border/70"
              >
                {/* Native `<details>` rather than an accordion component: it
                    is keyboard-accessible and announced as a disclosure for
                    free, needs no state, and `settings-hevy.tsx` already uses
                    the same element. Collapsed by default — seven exercises
                    of five set rows each was the reason this sheet ran four
                    screens long. `summariseSets` keeps a collapsed row
                    informative so folding costs nothing at a glance. */}
                <details className="group">
                  <summary className="flex cursor-pointer list-none items-center gap-3 p-4 [&::-webkit-details-marker]:hidden">
                    <div className="min-w-0 flex-1">
                      <h4 className="font-semibold">{exercise.title}</h4>
                      <p className="mt-0.5 font-mono text-xs text-muted-foreground">
                        {exercise.sets.length}{" "}
                        {exercise.sets.length === 1 ? "set" : "sets"}
                        {summariseSets(exercise.sets) &&
                          ` · ${summariseSets(exercise.sets)}`}
                      </p>
                    </div>
                    <ChevronDownIcon
                      aria-hidden="true"
                      className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180 motion-reduce:transition-none"
                    />
                  </summary>
                  <div className="px-4 pb-4">
                    {exercise.notes ? (
                      <p className="mb-3 whitespace-pre-wrap text-sm text-muted-foreground">
                        {exercise.notes}
                      </p>
                    ) : null}
                    {/* The 420px floor keeps the four columns from squashing
                        on desktop, but on a phone the body is ~310px, so it
                        forced a horizontal scroll on every set table and
                        clipped the Result column. Below sm the cells wrap. */}
                    <div className="sm:overflow-x-auto">
                      <table className="w-full text-left text-sm sm:min-w-[420px]">
                        <thead className="font-mono text-[11px] text-muted-foreground uppercase">
                          <tr>
                            <th className="pb-2 font-medium">Set</th>
                            <th className="pb-2 font-medium">Type</th>
                            <th className="pb-2 text-right font-medium">Result</th>
                            <th className="pb-2 text-right font-medium">RPE</th>
                          </tr>
                        </thead>
                        <tbody>
                          {exercise.sets.map((set) => (
                            <tr key={set.number} className="border-t border-border/50">
                              <td className="py-2.5 font-mono text-xs">{set.number}</td>
                              <td className="py-2.5 capitalize">{set.setType}</td>
                              <td className="py-2.5 text-right font-medium">
                                {formatSetResult(set)}
                              </td>
                              <td className="py-2.5 text-right text-muted-foreground">
                                {formatNumber(set.rpe)}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  </div>
                </details>
              </li>
            ))}
          </ol>
        )}
      </section>

      {/* A well: this previews generated Garmin text, so it reads as content
          sunk into the sheet rather than a panel raised off it. */}
      <section className="rounded-lg border border-border/70 bg-background p-4">
        <h3 className="mb-1 text-sm font-bold">Garmin description preview</h3>
        <p className="mb-3 text-xs leading-relaxed text-muted-foreground">
          Description only always uses this template. Merge and Replace follow your summary
          preference in Settings.
        </p>
        <p className="whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground">
          {detail.descriptionPreview}
        </p>
      </section>
    </div>
  );
}

function DetailValue({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="font-mono text-[11px] text-muted-foreground uppercase">{label}</p>
      <p className="mt-1 font-semibold">{value}</p>
    </div>
  );
}

export function formatWorkoutTime(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : new Intl.DateTimeFormat(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(date);
}

function formatDuration(start: string, end: string) {
  const seconds = Math.max(0, (new Date(end).getTime() - new Date(start).getTime()) / 1000);
  if (!Number.isFinite(seconds)) return "—";
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.round((seconds % 3600) / 60);
  return hours ? `${hours}h ${minutes}m` : `${minutes}m`;
}

function formatNumber(value: number | null) {
  return value == null
    ? "—"
    : new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(value);
}

function formatSeconds(value: number) {
  const minutes = Math.floor(value / 60);
  const seconds = Math.round(value % 60);
  return minutes ? `${minutes}m ${seconds}s` : `${seconds}s`;
}

function formatSetResult(set: HevyWorkoutSetDetail) {
  const parts: string[] = [];
  if (set.weightKg != null) parts.push(`${formatNumber(set.weightKg)} kg`);
  if (set.reps != null) parts.push(`${formatNumber(set.reps)} reps`);
  if (set.distanceMeters != null) parts.push(`${formatNumber(set.distanceMeters)} m`);
  if (set.durationSeconds != null) parts.push(formatSeconds(set.durationSeconds));
  if (set.customMetric != null) parts.push(String(set.customMetric));
  return parts.join(" · ") || "—";
}

/**
 * Collapses one dimension across an exercise's sets into `12 reps` when every
 * set matches, or `8–12 reps` when they don't — so a folded row still says
 * what was actually lifted.
 */
function summariseRange(
  values: number[],
  format: (value: number) => string,
  unit?: string,
): string | null {
  if (values.length === 0) return null;
  const min = Math.min(...values);
  const max = Math.max(...values);
  // The unit goes on the range, not on each end — "42–46 kg", never
  // "42 kg–46 kg". Durations pass no unit; they carry their own.
  const range = min === max ? format(min) : `${format(min)}–${format(max)}`;
  return unit ? `${range} ${unit}` : range;
}

/**
 * The one-line stand-in for a collapsed set table. Only dimensions the
 * exercise actually recorded appear — a bodyweight exercise has no weight,
 * a plank has no reps.
 *
 * ponytail: `customMetric` is left out. It has no unit to render and no
 * meaningful range, so it stays available in the expanded table only.
 */
function summariseSets(sets: HevyWorkoutSetDetail[]): string {
  const present = (pick: (set: HevyWorkoutSetDetail) => number | null) =>
    sets.map(pick).filter((value): value is number => value != null);

  return [
    summariseRange(present((set) => set.weightKg), formatNumber, "kg"),
    summariseRange(present((set) => set.reps), formatNumber, "reps"),
    summariseRange(present((set) => set.distanceMeters), formatNumber, "m"),
    summariseRange(present((set) => set.durationSeconds), formatSeconds),
  ]
    .filter((part): part is string => part !== null)
    .join(" · ");
}
