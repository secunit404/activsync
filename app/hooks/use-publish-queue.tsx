import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { toast } from "sonner";

import {
  invalidateActivityQueries,
  toastActionError,
  toastActionResult,
} from "@/hooks/use-activity-actions";
import {
  ApiError,
  cancelPublish,
  getPublishBatch,
  schedulePublishActivities,
  schedulePublishActivity,
  type PublishPhase,
  type PublishScheduled,
} from "@/lib/api";

/** Mirrors `publish_schedule.UNDO_WINDOW_SECONDS`; a pytest holds the two equal. */
export const PUBLISH_UNDO_WINDOW_MS = 5000;
const BATCH_POLL_MS = 1000;

export type { PublishPhase };

type QueuedPublish = {
  key: number;
  activityIds: readonly number[];
  phase: PublishPhase;
  cancel: () => void;
};

let queue: readonly QueuedPublish[] = [];
let phases: ReadonlyMap<number, PublishPhase> = new Map();
let nextKey = 0;
const listeners = new Set<() => void>();

function setQueue(next: readonly QueuedPublish[]) {
  queue = next;
  phases = new Map(
    next.flatMap((entry) => entry.activityIds.map((id) => [id, entry.phase] as const)),
  );
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getPhases() {
  return phases;
}

function toastNotPublished() {
  toast("Not published", { description: "Nothing was sent to Strava." });
}

function asError(error: unknown) {
  return error instanceof Error ? error : new Error(String(error));
}

/**
 * Publishing is the one action that changes the owner's Strava account, so it
 * never fires on the click itself. The click asks the server to schedule it;
 * the server holds it for the undo window and then sends it, so locking the
 * phone or closing the tab no longer loses a publish the owner confirmed.
 * This page only mirrors that: an Undo toast with a countdown, then the
 * outcome it reads back. The queue lives at module scope rather than in a
 * component so a publish started in the detail overlay survives the overlay
 * closing, and the list and the overlay agree on which rows are mid-publish.
 */
async function queuePublish(
  queryClient: QueryClient,
  activityIds: readonly number[],
  schedule: () => Promise<PublishScheduled>,
) {
  const key = nextKey++;
  const toastId = `publish-${key}`;
  let scheduled: PublishScheduled | null = null;
  let undoRequested = false;
  let undoing = false;
  let settled = false;
  let sendTimer: number | undefined = undefined;
  let pollTimer: number | undefined;

  const setPhase = (phase: PublishPhase) =>
    setQueue(queue.map((entry) => (entry.key === key ? { ...entry, phase } : entry)));

  // The refetch lands before the entry goes, so the rows never fall back to
  // the server state this page last read (still "waiting") in between.
  const settle = async (report: () => void) => {
    if (settled) {
      return;
    }
    settled = true;
    window.clearTimeout(sendTimer);
    window.clearTimeout(pollTimer);
    toast.dismiss(toastId);
    report();
    await invalidateActivityQueries(queryClient);
    setQueue(queue.filter((entry) => entry.key !== key));
  };

  const poll = async (batchId: string) => {
    if (settled) {
      return;
    }
    try {
      const batch = await getPublishBatch(batchId);
      if (batch.state === "done") {
        await settle(() => {
          if (batch.error) {
            toastActionError(new Error(batch.error));
          } else if (batch.result) {
            toastActionResult(batch.result);
          }
        });
        return;
      }
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) {
        await settle(toastNotPublished);
        return;
      }
    }
    pollTimer = window.setTimeout(() => void poll(batchId), BATCH_POLL_MS);
  };

  const startSending = (batchId: string) => {
    if (settled || queue.find((entry) => entry.key === key)?.phase === "sending") {
      return;
    }
    window.clearTimeout(sendTimer);
    toast.loading(publishingLabel(activityIds.length), {
      id: toastId,
      description: undefined,
      action: undefined,
    });
    setPhase("sending");
    void poll(batchId);
  };

  const cancel = async () => {
    if (settled || undoing || queue.find((entry) => entry.key === key)?.phase !== "waiting") {
      return;
    }
    if (scheduled === null) {
      undoRequested = true;
      return;
    }
    undoing = true;
    try {
      await cancelPublish(activityIds);
      await settle(toastNotPublished);
    } catch (error) {
      undoing = false;
      toastActionError(asError(error));
      if (error instanceof ApiError && error.status === 409) {
        startSending(scheduled.batchId);
      }
    }
  };

  setQueue([...queue, { key, activityIds, phase: "waiting", cancel: () => void cancel() }]);

  try {
    scheduled = await schedule();
  } catch (error) {
    await settle(() => toastActionError(asError(error)));
    return;
  }
  if (undoRequested) {
    await cancel();
    return;
  }

  const { batchId, remainingMs, undoWindowMs } = scheduled;
  const deadline = Date.now() + remainingMs;
  sendTimer = window.setTimeout(() => startSending(batchId), remainingMs);
  toast(publishingLabel(activityIds.length), {
    id: toastId,
    duration: Infinity,
    // Closing the toast would read as "cancel" but leave the publish queued;
    // Undo is the one way out, so the toast offers nothing else.
    closeButton: false,
    dismissible: false,
    description: <UndoCountdown deadline={deadline} windowMs={undoWindowMs} />,
    action: {
      label: "Undo",
      onClick: (event) => {
        // The toast stays up until the server confirms the undo.
        event.preventDefault();
        void cancel();
      },
    },
  });
}

/**
 * Undo for a publish this page did not start (another device, or this page
 * before a reload): the server state still shows it waiting.
 */
async function undoServerPublish(queryClient: QueryClient, activityId: number) {
  try {
    await cancelPublish([activityId]);
    toastNotPublished();
  } catch (error) {
    toastActionError(asError(error));
  } finally {
    await invalidateActivityQueries(queryClient);
  }
}

function publishingLabel(count: number) {
  return count === 1 ? "Publishing to Strava…" : `Publishing ${count} to Strava…`;
}

export function usePublishQueue() {
  const queryClient = useQueryClient();
  const queued = useSyncExternalStore(subscribe, getPhases, getPhases);
  const publishOne = useCallback(
    (activityId: number) =>
      void queuePublish(queryClient, [activityId], () => schedulePublishActivity(activityId)),
    [queryClient],
  );
  const publishMany = useCallback(
    (activityIds: number[]) =>
      void queuePublish(queryClient, activityIds, () => schedulePublishActivities(activityIds)),
    [queryClient],
  );
  const undo = useCallback(
    (activityId: number) => {
      const local = queue.find((entry) => entry.activityIds.includes(activityId));
      if (local) {
        local.cancel();
      } else {
        void undoServerPublish(queryClient, activityId);
      }
    },
    [queryClient],
  );
  return { queued, publishOne, publishMany, undo };
}

function UndoCountdown({ deadline, windowMs }: { deadline: number; windowMs: number }) {
  const [now, setNow] = useState(() => Date.now());
  const [drainMs] = useState(() => Math.max(0, deadline - Date.now()));
  useEffect(() => {
    const interval = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(interval);
  }, []);
  const remainingMs = Math.max(0, deadline - now);

  return (
    <span className="grid w-full gap-2">
      {/* The ticking number stays out of the live region, or every second
          would be re-announced; the static sentence below is read once. */}
      <span aria-hidden="true" className="font-mono tabular-nums">
        Sends in {Math.ceil(remainingMs / 1000)}s
      </span>
      <span className="sr-only">
        Sends in {windowMs / 1000} seconds unless you undo.
      </span>
      <span
        aria-hidden="true"
        className="block h-0.5 overflow-hidden rounded-full bg-border motion-reduce:hidden"
      >
        <span
          className="block h-full origin-left animate-[publish-undo-drain_linear_forwards] bg-primary"
          style={{ animationDuration: `${drainMs}ms` }}
        />
      </span>
    </span>
  );
}
