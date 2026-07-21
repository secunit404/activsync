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

const { runHevyQueueAction } = vi.hoisted(() => ({
  runHevyQueueAction: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, runHevyQueueAction };
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

function renderQueue(state: HevyQueueState) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <HevyQueue state={state} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

test("shows a quiet empty message, not the header badges, when the queue is empty", () => {
  renderQueue(emptyQueue());
  expect(screen.getByText(/nothing in the queue/i)).toBeInTheDocument();
  expect(screen.queryByText(/in flight/i)).not.toBeInTheDocument();
  expect(screen.queryByText(/problem/i)).not.toBeInTheDocument();
  expect(screen.queryByText(/skipped/i)).not.toBeInTheDocument();
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
  renderQueue(
    emptyQueue({
      counts: { inFlight: 0, problems: 1, skipped: 0 },
      problems: [item({ status: "failed", error: "Upload failed" })],
    }),
  );

  await userEvent.click(screen.getByRole("button", { name: "Skip" }));
  const dialog = screen.getByRole("alertdialog", { name: "Skip this workout?" });
  await userEvent.click(within(dialog).getByRole("button", { name: "Skip" }));

  await waitFor(() => expect(runHevyQueueAction).toHaveBeenCalledWith("hevy-1", "skip"));
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

test("in-flight items show their status but no actions", () => {
  renderQueue(
    emptyQueue({
      counts: { inFlight: 1, problems: 0, skipped: 0 },
      inFlight: [item({ status: "waiting_watch" })],
    }),
  );

  expect(screen.getByText("WAITING WATCH")).toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
