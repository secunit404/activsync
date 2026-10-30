import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { toast } from "sonner";

import {
  invalidateActivityQueries,
  toastActionError,
  toastActionResult,
} from "@/hooks/use-activity-actions";
import { publishActivities, publishActivity, type ActivityActionResult } from "@/lib/api";

export const PUBLISH_UNDO_WINDOW_MS = 5000;

export type PublishPhase = "waiting" | "sending";

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

/**
 * Publishing is the one action that changes the owner's Strava account, so it
 * never fires on the click itself: the request waits out
 * `PUBLISH_UNDO_WINDOW_MS` behind an Undo toast, and only an elapsed window
 * sends it. The queue lives at module scope rather than in a component so a
 * publish started in the detail overlay survives the overlay closing, and the
 * list and the overlay agree on which rows are mid-publish.
 */
function queuePublish(
  queryClient: QueryClient,
  activityIds: readonly number[],
  send: () => Promise<ActivityActionResult>,
) {
  const key = nextKey++;
  const toastId = `publish-${key}`;
  const deadline = Date.now() + PUBLISH_UNDO_WINDOW_MS;
  const remove = () => setQueue(queue.filter((entry) => entry.key !== key));

  // A plain in-page timer on purpose: closing or reloading the tab inside the
  // window drops it, so nothing is sent. The owner never saw it go through.
  const timer = window.setTimeout(async () => {
    toast.loading(publishingLabel(activityIds.length), {
      id: toastId,
      description: undefined,
      action: undefined,
    });
    setQueue(
      queue.map((entry) => (entry.key === key ? { ...entry, phase: "sending" } : entry)),
    );
    try {
      const result = await send();
      toast.dismiss(toastId);
      toastActionResult(result);
    } catch (error) {
      toast.dismiss(toastId);
      toastActionError(error instanceof Error ? error : new Error(String(error)));
    } finally {
      await invalidateActivityQueries(queryClient);
      remove();
    }
  }, PUBLISH_UNDO_WINDOW_MS);

  const cancel = () => {
    if (queue.find((entry) => entry.key === key)?.phase !== "waiting") {
      return;
    }
    window.clearTimeout(timer);
    remove();
    toast.dismiss(toastId);
    toast("Not published", { description: "Nothing was sent to Strava." });
  };

  setQueue([...queue, { key, activityIds, phase: "waiting", cancel }]);

  toast(publishingLabel(activityIds.length), {
    id: toastId,
    duration: Infinity,
    // Closing the toast would read as "cancel" but leave the publish queued;
    // Undo is the one way out, so the toast offers nothing else.
    closeButton: false,
    dismissible: false,
    description: <UndoCountdown deadline={deadline} />,
    action: { label: "Undo", onClick: cancel },
  });
}

function undoPublish(activityId: number) {
  queue
    .find((entry) => entry.phase === "waiting" && entry.activityIds.includes(activityId))
    ?.cancel();
}

function publishingLabel(count: number) {
  return count === 1 ? "Publishing to Strava…" : `Publishing ${count} to Strava…`;
}

export function usePublishQueue() {
  const queryClient = useQueryClient();
  const queued = useSyncExternalStore(subscribe, getPhases, getPhases);
  const publishOne = useCallback(
    (activityId: number) =>
      queuePublish(queryClient, [activityId], () => publishActivity(activityId)),
    [queryClient],
  );
  const publishMany = useCallback(
    (activityIds: number[]) =>
      queuePublish(queryClient, activityIds, () => publishActivities(activityIds)),
    [queryClient],
  );
  return { queued, publishOne, publishMany, undo: undoPublish };
}

function UndoCountdown({ deadline }: { deadline: number }) {
  const [now, setNow] = useState(() => Date.now());
  const [drainMs] = useState(() => Math.max(0, deadline - Date.now()));
  useEffect(() => {
    const interval = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(interval);
  }, []);
  const remainingMs = Math.max(0, deadline - now);

  return (
    <span className="grid w-40 max-w-full gap-2">
      {/* The ticking number stays out of the live region, or every second
          would be re-announced; the static sentence below is read once. */}
      <span aria-hidden="true" className="font-mono tabular-nums">
        Sends in {Math.ceil(remainingMs / 1000)}s
      </span>
      <span className="sr-only">
        Sends in {PUBLISH_UNDO_WINDOW_MS / 1000} seconds unless you undo.
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
