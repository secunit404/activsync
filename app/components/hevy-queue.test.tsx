import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { HevyQueueItem, HevyQueueState } from "@/lib/api";
import { HevyQueue } from "./hevy-queue";

beforeEach(() => {
  vi.clearAllMocks();
});

const { scheduleHevyMatch, cancelHevyMatch, getHevyMatchStatus, getHevyWorkout, runHevyQueueAction } =
  vi.hoisted(() => ({
    scheduleHevyMatch: vi.fn(),
    cancelHevyMatch: vi.fn(),
    getHevyMatchStatus: vi.fn(),
    getHevyWorkout: vi.fn(),
    runHevyQueueAction: vi.fn(),
  }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    scheduleHevyMatch,
    cancelHevyMatch,
    getHevyMatchStatus,
    getHevyWorkout,
    runHevyQueueAction,
  };
});

const { toastSuccess, toastError, toastBase } = vi.hoisted(() => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
  toastBase: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: Object.assign(toastBase, {
    success: toastSuccess,
    error: toastError,
    loading: vi.fn(),
    dismiss: vi.fn(),
  }),
}));

function item(overrides: Partial<HevyQueueItem>): HevyQueueItem {
  return {
    hevyId: "hevy-1",
    title: "Leg Day",
    status: "syncing",
    error: null,
    startDisplay: "Jul 20, 06:12",
    needsMapping: false,
    hasOpenOperation: false,
    resyncable: false,
    awaitingMatch: false,
    matchedGarminActivityId: null,
    matchedGarminTitle: null,
    matchedStravaActivityId: null,
    matchedStravaUrl: null,
    matchScheduledAt: null,
    matchPhase: null,
    ...overrides,
  };
}

function emptyQueue(overrides: Partial<HevyQueueState> = {}): HevyQueueState {
  return {
    enabled: true,
    counts: { inFlight: 0, problems: 0, skipped: 0 },
    inFlight: [],
    problems: [],
    skipped: [],
    ...overrides,
  };
}

function renderQueue(state: HevyQueueState, onQueueChanged?: () => void) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <HevyQueue state={state} onQueueChanged={onQueueChanged} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

test("shows a quiet empty message, not the header badges, when the queue is empty", () => {
  renderQueue(emptyQueue());
  expect(screen.getByText(/nothing needs attention/i)).toBeInTheDocument();
  expect(screen.queryByText(/pending/i)).not.toBeInTheDocument();
  expect(screen.queryByText(/^\d+ needs attention$/i)).not.toBeInTheDocument();
  expect(screen.queryByText(/skipped/i)).not.toBeInTheDocument();
});

// The header carries no counts: every row states its own status, so the
// badges only restated what the list already said.
test("the header is the title alone, with no count badges", () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 2, problems: 1, skipped: 1 },
    }),
  );

  expect(screen.getByText("Sync queue")).toBeInTheDocument();
  expect(screen.queryByText("2 pending")).not.toBeInTheDocument();
  expect(screen.queryByText("1 needs attention")).not.toBeInTheDocument();
  expect(screen.queryByText("1 skipped")).not.toBeInTheDocument();
});

test("an imported Garmin match says queued while automatic processing starts", () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [
        item({
          status: "waiting_watch",
          matchedGarminActivityId: 123,
          matchedGarminTitle: "Morning strength",
        }),
      ],
    }),
  );

  // The status reads once, on the row's detail line — there is no second
  // badge repeating it next to the actions.
  expect(screen.getByText("Queued")).toBeVisible();
  expect(screen.queryByText(/waiting for garmin/i)).not.toBeInTheDocument();
});

test("retry calls the queue action and reports success via toast", async () => {
  runHevyQueueAction.mockResolvedValue({ message: "Retry queued for Leg Day." });
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [item({ status: "failed", error: "Upload failed", hasOpenOperation: false })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Retry Leg Day" }));

  await waitFor(() =>
    expect(runHevyQueueAction).toHaveBeenCalledWith("hevy-1", "retry"),
  );
  await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith("Retry queued for Leg Day."));
});

test("an awaiting match offers per-workout strategies", () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [
        item({
          status: "awaiting_match",
          awaitingMatch: true,
          matchedGarminActivityId: 123,
          matchedGarminTitle: "Morning strength",
        }),
      ],
    }),
  );

  expect(screen.getByText(/matched morning strength/i)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Merge Leg Day" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Replace Leg Day" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Description only Leg Day" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Skip Leg Day" })).toBeInTheDocument();
});

test("loads and shows the actual Hevy workout only after opening details", async () => {
  getHevyWorkout.mockResolvedValue({
    hevyId: "hevy-1",
    title: "Leg Day",
    startTime: "2026-07-20T06:12:00Z",
    endTime: "2026-07-20T07:20:00Z",
    notes: "Controlled tempo\nFelt good",
    exercises: [
      {
        title: "Back Squat (Barbell)",
        notes: "Three-second descent",
        templateId: "squat-1",
        sets: [
          {
            number: 1,
            setType: "normal",
            reps: 8,
            weightKg: 100,
            distanceMeters: null,
            durationSeconds: null,
            rpe: 8,
            customMetric: null,
          },
        ],
      },
    ],
    descriptionPreview: "🏋️ Leg Day\n\n• Back Squat (Barbell): 1 set - 100.0kg × 8",
  });
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [item({ status: "awaiting_match", awaitingMatch: true })],
    }),
  );

  expect(getHevyWorkout).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "View workout Leg Day" }));

  const dialog = await screen.findByRole("dialog", { name: "Leg Day" });
  expect(getHevyWorkout).toHaveBeenCalledWith("hevy-1", expect.any(AbortSignal));
  expect(within(dialog).getByText("Back Squat (Barbell)")).toBeInTheDocument();
  expect(within(dialog).getByRole("cell", { name: "100 kg, 8 reps" })).toBeInTheDocument();
  expect(within(dialog).getByText("Controlled tempo", { exact: false })).toBeInTheDocument();
  expect(within(dialog).getByText(/Description only always uses/i)).toBeInTheDocument();
});

const scheduled = {
  deadline: "2026-07-22T12:00:05+00:00",
  strategy: "merge",
  remainingMs: 5000,
  undoWindowMs: 5000,
};

// The queue of scheduled decisions lives at module scope, so a test that
// schedules one must undo it or the next test starts with that row locked.
async function undoAndSettle() {
  cancelHevyMatch.mockResolvedValue({ cancelled: true });
  await userEvent.click(await screen.findByRole("button", { name: "Undo Leg Day" }));
  expect(await screen.findByRole("button", { name: "Merge Leg Day" })).toBeEnabled();
}

function awaitingQueue(overrides: Partial<HevyQueueItem> = {}) {
  return emptyQueue({
    counts: { inFlight: 1, problems: 0, skipped: 0 },
    inFlight: [item({ status: "awaiting_match", awaitingMatch: true, ...overrides })],
  });
}

test("merge schedules the choice instead of applying it, and offers Undo", async () => {
  scheduleHevyMatch.mockResolvedValue(scheduled);
  renderQueue(awaitingQueue());

  await userEvent.click(screen.getByRole("button", { name: "Merge Leg Day" }));

  await waitFor(() => expect(scheduleHevyMatch).toHaveBeenCalledWith("hevy-1", "merge"));
  expect(screen.getByRole("button", { name: "Undo Leg Day" })).toBeEnabled();
  expect(screen.queryByRole("button", { name: "Merge Leg Day" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Skip Leg Day" })).not.toBeInTheDocument();
  expect(screen.getByText(/Applying shortly/)).toBeVisible();
  await undoAndSettle();
});

test("description only schedules straight away too", async () => {
  scheduleHevyMatch.mockResolvedValue({ ...scheduled, strategy: "describe" });
  renderQueue(awaitingQueue());

  await userEvent.click(screen.getByRole("button", { name: "Description only Leg Day" }));

  await waitFor(() => expect(scheduleHevyMatch).toHaveBeenCalledWith("hevy-1", "describe"));
  await undoAndSettle();
});

test("Undo cancels the scheduled choice and puts the strategy buttons back", async () => {
  scheduleHevyMatch.mockResolvedValue(scheduled);
  cancelHevyMatch.mockResolvedValue({ cancelled: true });
  renderQueue(awaitingQueue());

  await userEvent.click(screen.getByRole("button", { name: "Merge Leg Day" }));
  await userEvent.click(await screen.findByRole("button", { name: "Undo Leg Day" }));

  await waitFor(() => expect(cancelHevyMatch).toHaveBeenCalledWith("hevy-1"));
  expect(await screen.findByRole("button", { name: "Merge Leg Day" })).toBeEnabled();
});

test("a decision scheduled elsewhere shows Undo, and Undo cancels it on the server", async () => {
  cancelHevyMatch.mockResolvedValue({ cancelled: true });
  renderQueue(
    awaitingQueue({ matchPhase: "waiting", matchScheduledAt: "2026-07-22T12:00:05+00:00" }),
  );

  expect(screen.queryByRole("button", { name: "Merge Leg Day" })).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Undo Leg Day" }));

  await waitFor(() => expect(cancelHevyMatch).toHaveBeenCalledWith("hevy-1"));
  expect(scheduleHevyMatch).not.toHaveBeenCalled();
});

test("a decision being applied shows a disabled Updating Garmin state", () => {
  renderQueue(awaitingQueue({ matchPhase: "applying" }));

  expect(screen.getByRole("button", { name: "Updating Garmin… Leg Day" })).toBeDisabled();
  expect(screen.queryByRole("button", { name: "Undo Leg Day" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Merge Leg Day" })).not.toBeInTheDocument();
  expect(screen.getAllByText("Updating Garmin…").length).toBeGreaterThan(0);
});

test("replace asks for confirmation before scheduling", async () => {
  scheduleHevyMatch.mockResolvedValue({ ...scheduled, strategy: "replace" });
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [item({ status: "awaiting_match", awaitingMatch: true })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Replace Leg Day" }));
  const dialog = screen.getByRole("alertdialog", {
    name: "Replace this Garmin workout?",
  });
  expect(scheduleHevyMatch).not.toHaveBeenCalled();
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Replace Garmin workout" }),
  );

  await waitFor(() => expect(scheduleHevyMatch).toHaveBeenCalledWith("hevy-1", "replace"));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  await undoAndSettle();
});

test("an already-published match links Strava and withholds Replace", () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [
        item({
          status: "awaiting_match",
          awaitingMatch: true,
          matchedGarminActivityId: 111,
          matchedGarminTitle: "Morning strength",
          matchedStravaActivityId: 222,
          matchedStravaUrl: "https://www.strava.com/activities/222",
        }),
      ],
    }),
  );

  expect(screen.getByText(/Already on Strava/)).toBeVisible();
  const strava = screen.getByRole("link", { name: "View on Strava" });
  expect(strava).toHaveAttribute("href", "https://www.strava.com/activities/222");
  expect(strava).toHaveClass("font-bold");
  expect(screen.queryByRole("button", { name: "Replace Leg Day" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Merge Leg Day" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Description only Leg Day" })).toBeVisible();
});

test("skip opens a confirmation dialog with the real copy, and does nothing when cancelled", async () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [item({ status: "failed", error: "Upload failed" })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Skip Leg Day" }));

  const dialog = screen.getByRole("alertdialog", { name: "Skip this workout?" });
  expect(dialog).toHaveAccessibleDescription("Skip Leg Day?");

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  expect(runHevyQueueAction).not.toHaveBeenCalled();
});

test("skip runs the action once confirmed in the dialog", async () => {
  runHevyQueueAction.mockResolvedValue({ message: "Skipped Leg Day." });
  const onQueueChanged = vi.fn();
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [item({ status: "failed", error: "Upload failed" })],
    }),
    onQueueChanged,
  );

  await userEvent.click(screen.getByRole("button", { name: "Skip Leg Day" }));
  const dialog = screen.getByRole("alertdialog", { name: "Skip this workout?" });
  await userEvent.click(within(dialog).getByRole("button", { name: "Skip" }));

  await waitFor(() => expect(runHevyQueueAction).toHaveBeenCalledWith("hevy-1", "skip"));
  expect(onQueueChanged).toHaveBeenCalledOnce();
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
});

test("unskip is offered for skipped items and calls the unskip action", async () => {
  runHevyQueueAction.mockResolvedValue({ message: "Restored Mobility Flow." });
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 0, skipped: 1 },
      skipped: [item({ hevyId: "hevy-2", title: "Mobility Flow", status: "skipped" })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Unskip Mobility Flow" }));

  await waitFor(() => expect(runHevyQueueAction).toHaveBeenCalledWith("hevy-2", "unskip"));
});

test("resync-fresh is only offered when the item is resyncable, and asks for confirmation", async () => {
  runHevyQueueAction.mockResolvedValue({ message: "Fresh sync queued." });
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [
        item({
          hevyId: "hevy-3",
          status: "needs_review",
          error: "deleted on Garmin",
          resyncable: true,
        }),
      ],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Re-sync fresh Leg Day" }));

  const dialog = screen.getByRole("alertdialog", { name: "Re-sync this workout?" });
  expect(dialog).toHaveAccessibleDescription(
    "Re-sync Leg Day as a fresh Garmin upload?",
  );

  await userEvent.click(within(dialog).getByRole("button", { name: "Re-sync fresh" }));

  await waitFor(() =>
    expect(runHevyQueueAction).toHaveBeenCalledWith("hevy-3", "resync-fresh"),
  );
});

test("a needs-mapping problem is flagged inline rather than linking (no templateId to link to)", () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [item({ needsMapping: true, error: "unmapped exercises: Cable Fly" })],
    }),
  );

  expect(screen.getByText(/needs mapping/i)).toBeInTheDocument();
  expect(screen.queryByRole("link")).not.toBeInTheDocument();
});

test("skip is withheld while an operation is already open for the item", () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [item({ status: "failed", error: "Upload failed", hasOpenOperation: true })],
    }),
  );

  expect(screen.queryByRole("button", { name: "Skip Leg Day" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Retry Leg Day" })).toBeInTheDocument();
});

test("reports a failed action via an error toast", async () => {
  runHevyQueueAction.mockRejectedValue(new Error("Workout is being processed right now."));
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [item({ status: "failed", error: "Upload failed" })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Retry Leg Day" }));

  await waitFor(() =>
    expect(toastError).toHaveBeenCalledWith(
      "Workout is being processed right now.",
      expect.objectContaining({ duration: expect.any(Number) }),
    ),
  );
});

test("in-flight items show their status and inspection but no mutation actions", () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [item({ status: "waiting_watch" })],
    }),
  );

  expect(screen.getByText("Waiting for Garmin")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "View workout Leg Day" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Retry Leg Day" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Skip Leg Day" })).not.toBeInTheDocument();
});
