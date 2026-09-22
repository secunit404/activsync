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

const { chooseHevyMatch, getHevyWorkout, runHevyQueueAction } = vi.hoisted(() => ({
  chooseHevyMatch: vi.fn(),
  getHevyWorkout: vi.fn(),
  runHevyQueueAction: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, chooseHevyMatch, getHevyWorkout, runHevyQueueAction };
});

const { toastSuccess, toastError } = vi.hoisted(() => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: { success: toastSuccess, error: toastError },
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
  expect(screen.getByText(/· Queued$/)).toBeVisible();
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

  await userEvent.click(screen.getByRole("button", { name: "Retry" }));

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
  expect(screen.getByRole("button", { name: "Merge" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Replace" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Description only" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Skip" })).toBeInTheDocument();
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
    descriptionPreview: "🏋️ Leg Day\n\n• Back Squat (Barbell): 1 set · 100.0kg × 8",
  });
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [item({ status: "awaiting_match", awaitingMatch: true })],
    }),
  );

  expect(getHevyWorkout).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "View workout" }));

  const dialog = await screen.findByRole("dialog", { name: "Leg Day" });
  expect(getHevyWorkout).toHaveBeenCalledWith("hevy-1", expect.any(AbortSignal));
  expect(within(dialog).getByText("Back Squat (Barbell)")).toBeInTheDocument();
  expect(within(dialog).getByText("100 kg · 8 reps")).toBeInTheDocument();
  expect(within(dialog).getByText("Controlled tempo", { exact: false })).toBeInTheDocument();
  expect(within(dialog).getByText(/Description only always uses/i)).toBeInTheDocument();
});

test("merge applies the selected strategy to the awaiting workout", async () => {
  chooseHevyMatch.mockResolvedValue({ message: "Merged Leg Day." });
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [item({ status: "awaiting_match", awaitingMatch: true })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Merge" }));

  await waitFor(() =>
    expect(chooseHevyMatch).toHaveBeenCalledWith("hevy-1", "merge"),
  );
});

test("replace asks for confirmation before applying", async () => {
  chooseHevyMatch.mockResolvedValue({ message: "Replaced Leg Day." });
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [item({ status: "awaiting_match", awaitingMatch: true })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Replace" }));
  const dialog = screen.getByRole("alertdialog", {
    name: "Replace this Garmin workout?",
  });
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Replace Garmin workout" }),
  );

  await waitFor(() =>
    expect(chooseHevyMatch).toHaveBeenCalledWith("hevy-1", "replace"),
  );
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
  expect(screen.getByRole("link", { name: /Strava/ })).toHaveAttribute(
    "href",
    "https://www.strava.com/activities/222",
  );
  expect(screen.queryByRole("button", { name: "Replace" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Merge" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Description only" })).toBeVisible();
});

test("skip opens a confirmation dialog with the real copy, and does nothing when cancelled", async () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [item({ status: "failed", error: "Upload failed" })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Skip" }));

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

  await userEvent.click(screen.getByRole("button", { name: "Skip" }));
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

  await userEvent.click(screen.getByRole("button", { name: "Unskip" }));

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

  await userEvent.click(screen.getByRole("button", { name: "Re-sync fresh" }));

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

  expect(screen.queryByRole("button", { name: "Skip" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
});

test("reports a failed action via an error toast", async () => {
  runHevyQueueAction.mockRejectedValue(new Error("Workout is being processed right now."));
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [item({ status: "failed", error: "Upload failed" })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Retry" }));

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

  expect(screen.getByText(/· Waiting for Garmin$/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "View workout" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Retry" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Skip" })).not.toBeInTheDocument();
});
