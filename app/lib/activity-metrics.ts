import type { Activity } from "@/lib/api";

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
