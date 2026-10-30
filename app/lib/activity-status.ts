import type { ActivityView } from "@/lib/activity-view";
import type { Activity, PollIntervals } from "@/lib/api";

function withinMinutes(minutes: number): string {
  return minutes <= 1 ? "within a minute" : `within ${minutes} min`;
}

/**
 * One line on what a status (or filter) means for the owner. Pending names
 * the Strava check interval when it is known, since that is when it goes out,
 * and promises nothing while the Strava connection is broken.
 */
export function statusDescription(
  view: ActivityView,
  intervals?: PollIntervals,
  stravaBroken = false,
): string {
  switch (view) {
    case "review":
      return "Held or removed from Strava. These wait for you.";
    case "pending":
      if (stravaBroken) {
        return "Publishes to Strava once Strava is reconnected.";
      }
      return intervals
        ? `Publishes to Strava on its own, usually ${withinMinutes(intervals.stravaMinutes)}.`
        : "Publishes to Strava on its own at the next check.";
    case "held":
      return "Waits for you. Nothing reaches Strava until you publish.";
    case "missing":
      return "Removed from Strava after publishing. Republish or exclude.";
    case "published":
      return "On Strava, uploaded by ActivSync or already there.";
    case "excluded":
      return "Kept off Strava for good, unless you restore it.";
    case "all":
      return "Every activity synced from Garmin.";
  }
}

/**
 * Why a row is waiting on the owner, short enough for one truncated line
 * under its status. Null for statuses that need nothing from them.
 */
export function waitingReason(activity: Pick<Activity, "publishStatus" | "holdReason">): string | null {
  if (activity.publishStatus === "missing") {
    return "Removed from Strava after publishing";
  }
  if (activity.publishStatus !== "held") {
    return null;
  }
  switch (activity.holdReason ?? "category") {
    case "category":
      return "Auto-sync is off for this type";
    case "backlog":
      return "Older activity found after a reconnect";
    default:
      return "Waiting for your review";
  }
}

/**
 * The list's reason line, kept for the surprising cases only: a category
 * hold is what the owner configured, so it is left to the detail header.
 */
export function rowReason(activity: Pick<Activity, "publishStatus" | "holdReason">): string | null {
  const surprising =
    activity.publishStatus === "missing" ||
    (activity.publishStatus === "held" && activity.holdReason === "backlog");
  return surprising ? waitingReason(activity) : null;
}

/** The detail header's line: why it waits, or what its status means. */
export function activityExplanation(
  activity: Pick<Activity, "publishStatus" | "holdReason">,
  intervals?: PollIntervals,
  stravaBroken = false,
): string {
  if (activity.publishStatus === "held") {
    return waitingReason(activity) ?? statusDescription("held", intervals);
  }
  return statusDescription(activity.publishStatus, intervals, stravaBroken);
}
