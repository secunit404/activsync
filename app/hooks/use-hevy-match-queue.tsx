import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useCallback, useSyncExternalStore } from "react";
import { toast } from "sonner";

import { UndoCountdown } from "@/components/undo-countdown";
import {
  ApiError,
  cancelHevyMatch,
  getHevyMatchStatus,
  scheduleHevyMatch,
  type HevyMatchPhase,
  type HevyMatchScheduled,
  type HevyMatchStrategy,
} from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { ERROR_TOAST_DURATION_MS } from "@/lib/toast-duration";

/** Mirrors `hevy_match_schedule.UNDO_WINDOW_SECONDS`; a pytest holds the two equal. */
export const HEVY_MATCH_UNDO_WINDOW_MS = 5000;
const STATUS_POLL_MS = 1000;

export type { HevyMatchPhase };

type QueuedMatch = {
  key: number;
  hevyId: string;
  phase: HevyMatchPhase;
  cancel: () => void;
};

let queue: readonly QueuedMatch[] = [];
let phases: ReadonlyMap<string, HevyMatchPhase> = new Map();
let nextKey = 0;
const listeners = new Set<() => void>();

function setQueue(next: readonly QueuedMatch[]) {
  queue = next;
  phases = new Map(next.map((entry) => [entry.hevyId, entry.phase] as const));
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

function toastNotChanged() {
  toast("Not changed", { description: "Garmin was left as it was." });
}

function toastFailure(error: Error) {
  toast.error(error.message, { duration: ERROR_TOAST_DURATION_MS });
}

function asError(error: unknown) {
  return error instanceof Error ? error : new Error(String(error));
}

async function invalidateQueue(queryClient: QueryClient) {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.hevyQueue }),
    queryClient.invalidateQueries({ queryKey: queryKeys.appState }),
    queryClient.invalidateQueries({ queryKey: queryKeys.allActivities }),
  ]);
}

const PENDING_LABELS: Record<HevyMatchStrategy, (title: string) => string> = {
  merge: (title) => `Merging ${title} into Garmin…`,
  replace: (title) => `Replacing ${title} on Garmin…`,
  describe: (title) => `Updating ${title}'s description…`,
};

/**
 * Choosing how a Hevy workout lands on Garmin rewrites an activity in the
 * owner's Garmin account, so it never fires on the click itself. The click
 * asks the server to schedule it; the server holds it for the undo window and
 * then applies it, so closing the tab does not lose a decision the owner made.
 * This page only mirrors that: an Undo toast with a countdown, then the
 * outcome it reads back. The queue lives at module scope so the toast and the
 * row agree on which workouts are mid-decision.
 */
async function queueMatch(
  queryClient: QueryClient,
  hevyId: string,
  title: string,
  strategy: HevyMatchStrategy,
  onApplied: (() => void) | undefined,
) {
  const key = nextKey++;
  const toastId = `hevy-match-${key}`;
  const label = PENDING_LABELS[strategy](title);
  let scheduled: HevyMatchScheduled | null = null;
  let undoRequested = false;
  let undoing = false;
  let settled = false;
  let applyTimer: number | undefined = undefined;
  let pollTimer: number | undefined;

  const setPhase = (phase: HevyMatchPhase) =>
    setQueue(queue.map((entry) => (entry.key === key ? { ...entry, phase } : entry)));

  const settle = async (report: () => void) => {
    if (settled) {
      return;
    }
    settled = true;
    window.clearTimeout(applyTimer);
    window.clearTimeout(pollTimer);
    toast.dismiss(toastId);
    report();
    await invalidateQueue(queryClient);
    setQueue(queue.filter((entry) => entry.key !== key));
  };

  const poll = async () => {
    if (settled) {
      return;
    }
    try {
      const status = await getHevyMatchStatus(hevyId);
      if (status.state === "done") {
        await settle(() => {
          toast.success(status.message ?? `Updated ${title}.`);
          onApplied?.();
        });
        return;
      }
      if (status.state === "failed") {
        await settle(() => toastFailure(new Error(status.message ?? "Could not update Garmin.")));
        return;
      }
      if (status.state === "skipped") {
        await settle(() =>
          toast("Not changed", { description: `${title} was already handled.` }),
        );
        return;
      }
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) {
        await settle(toastNotChanged);
        return;
      }
    }
    pollTimer = window.setTimeout(() => void poll(), STATUS_POLL_MS);
  };

  const startApplying = () => {
    if (settled || queue.find((entry) => entry.key === key)?.phase === "applying") {
      return;
    }
    window.clearTimeout(applyTimer);
    toast.loading("Updating Garmin…", { id: toastId, description: undefined, action: undefined });
    setPhase("applying");
    void poll();
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
      await cancelHevyMatch(hevyId);
      await settle(toastNotChanged);
    } catch (error) {
      undoing = false;
      toastFailure(asError(error));
      if (error instanceof ApiError && error.status === 409) {
        startApplying();
      }
    }
  };

  setQueue([...queue, { key, hevyId, phase: "waiting", cancel: () => void cancel() }]);

  try {
    scheduled = await scheduleHevyMatch(hevyId, strategy);
  } catch (error) {
    await settle(() => toastFailure(asError(error)));
    return;
  }
  if (undoRequested) {
    await cancel();
    return;
  }

  const { remainingMs, undoWindowMs } = scheduled;
  const deadline = Date.now() + remainingMs;
  applyTimer = window.setTimeout(startApplying, remainingMs);
  toast(label, {
    id: toastId,
    duration: Infinity,
    // Closing the toast would read as "cancel" but leave the decision queued;
    // Undo is the one way out, so the toast offers nothing else.
    closeButton: false,
    dismissible: false,
    description: <UndoCountdown deadline={deadline} windowMs={undoWindowMs} verb="Applies" />,
    action: {
      label: "Undo",
      onClick: (event) => {
        event.preventDefault();
        void cancel();
      },
    },
  });
}

/**
 * Undo for a decision this page did not start (another device, or this page
 * before a reload): the server state still shows it waiting.
 */
async function undoServerMatch(queryClient: QueryClient, hevyId: string) {
  try {
    await cancelHevyMatch(hevyId);
    toastNotChanged();
  } catch (error) {
    toastFailure(asError(error));
  } finally {
    await invalidateQueue(queryClient);
  }
}

export function useHevyMatchQueue() {
  const queryClient = useQueryClient();
  const queued = useSyncExternalStore(subscribe, getPhases, getPhases);
  const choose = useCallback(
    (
      hevyId: string,
      title: string,
      strategy: HevyMatchStrategy,
      onApplied?: () => void,
    ) => void queueMatch(queryClient, hevyId, title, strategy, onApplied),
    [queryClient],
  );
  const undo = useCallback(
    (hevyId: string) => {
      const local = queue.find((entry) => entry.hevyId === hevyId);
      if (local) {
        local.cancel();
      } else {
        void undoServerMatch(queryClient, hevyId);
      }
    },
    [queryClient],
  );
  return { queued, choose, undo };
}
