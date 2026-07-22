import type { HevyWorkoutDetail, HevyWorkoutSetDetail } from "@/lib/api";

export function HevyWorkoutDetailContent({ detail }: { detail: HevyWorkoutDetail }) {
  return (
    <div className="space-y-6">
      <div className="grid grid-cols-2 gap-3 rounded-lg border border-border/70 bg-muted/25 p-4 text-sm">
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
                className="rounded-lg border border-border/70 p-4"
              >
                <div className="mb-3 flex items-baseline justify-between gap-3">
                  <h4 className="font-semibold">{exercise.title}</h4>
                  <span className="shrink-0 font-mono text-xs text-muted-foreground">
                    {exercise.sets.length} {exercise.sets.length === 1 ? "set" : "sets"}
                  </span>
                </div>
                {exercise.notes ? (
                  <p className="mb-3 whitespace-pre-wrap text-sm text-muted-foreground">
                    {exercise.notes}
                  </p>
                ) : null}
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[420px] text-left text-sm">
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
              </li>
            ))}
          </ol>
        )}
      </section>

      <section className="rounded-lg border border-border/70 bg-muted/20 p-4">
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

function formatSetResult(set: HevyWorkoutSetDetail) {
  const parts: string[] = [];
  if (set.weightKg != null) parts.push(`${formatNumber(set.weightKg)} kg`);
  if (set.reps != null) parts.push(`${formatNumber(set.reps)} reps`);
  if (set.distanceMeters != null) parts.push(`${formatNumber(set.distanceMeters)} m`);
  if (set.durationSeconds != null) {
    const minutes = Math.floor(set.durationSeconds / 60);
    const seconds = Math.round(set.durationSeconds % 60);
    parts.push(minutes ? `${minutes}m ${seconds}s` : `${seconds}s`);
  }
  if (set.customMetric != null) parts.push(String(set.customMetric));
  return parts.join(" · ") || "—";
}
