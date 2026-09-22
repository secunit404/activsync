import type { Activity, PublishStatus } from "@/lib/api";

/**
 * The table/card EFFORT figure is whichever intensity metric the activity
 * actually recorded: heart rate for cardio, power for a ride, lifted volume
 * for a Hevy strength session. The backend only ever populates one of these
 * three `ActivityDetail` fields per activity (the others come back as `""`),
 * so taking the first non-empty one in this priority order is safe and
 * matches the design handoff's mixed "156 bpm" / "198 W" / "8,420 kg" column.
 */
export function activityEffort(activity: Activity): string {
  const { avgHr, avgPower, totalVolume } = activity.detail;
  return avgHr || avgPower || totalVolume || "";
}

/**
 * The full metric set for the activity detail screen's stat grid (handoff
 * frames 2a/2b), ordered to match the handoff's own priority (distance,
 * duration, pace/speed, effort, then strength-specific totals last). Unlike
 * `activityEffort` (one figure, first match wins) this is exhaustive: a
 * strength session and a run populate disjoint subsets of `ActivityDetail`,
 * so every non-empty field is shown rather than guessing which type the
 * activity is.
 */
export function activityDetailMetrics(activity: Activity): Array<[string, string]> {
  const detail = activity.detail;
  return (
    [
      ["DISTANCE", detail.distance],
      ["DURATION", detail.duration],
      ["MOVING", detail.movingTime],
      ["ELAPSED", detail.elapsedTime],
      ["PACE", detail.pace],
      ["SPEED", detail.speed],
      ["ELEV GAIN", detail.elevGain],
      ["ELEV LOSS", detail.elevLoss],
      ["CALORIES", detail.calories],
      ["AVG HR", detail.avgHr],
      ["MAX HR", detail.maxHr],
      ["AVG POWER", detail.avgPower],
      ["MAX POWER", detail.maxPower],
      ["NORM POWER", detail.normPower],
      ["AEROBIC TE", detail.aerobicTe],
      ["ANAEROBIC TE", detail.anaerobicTe],
      ["TRAINING LOAD", detail.trainingLoad],
      ["CADENCE", detail.avgCadence],
      ["MAX CADENCE", detail.maxCadence],
      ["SETS", detail.totalSets],
      ["REPS", detail.totalReps],
      ["VOLUME", detail.totalVolume],
    ] satisfies Array<[string, string]>
  ).filter((metric): metric is [string, string] => Boolean(metric[1]));
}

/**
 * Whether an activity can still be published (or re-published) to Strava —
 * the detail screen's footer (frame 2a) only offers Publish/Exclude for
 * these three statuses. `excluded` gets Restore instead (see
 * `ActivityDetailViewFooter`); `published` has nothing left to offer beyond
 * Edit.
 */
export function isPublishableStatus(status: PublishStatus): boolean {
  return status === "pending" || status === "held" || status === "missing";
}

/**
 * Excludable per the server's own rule — `api_routes.py`'s exclude endpoint
 * answers 409 "Only pending, held, or missing activities can be excluded"
 * for anything else.
 *
 * This shares its predicate with `isPublishableStatus` today. It stays a
 * separate function rather than an alias so that changing one rule later
 * cannot silently move the other: they are different questions that happen
 * to have the same answer.
 */
export function isExcludableStatus(status: PublishStatus): boolean {
  return status === "pending" || status === "held" || status === "missing";
}
