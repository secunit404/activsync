import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { expect, test } from "vitest";

import type { AppState, HevyQueueState, HevyToolsState } from "@/lib/api";
import { HevyView } from "./hevy";

const connectedAppState: Pick<AppState, "hevy"> = {
  hevy: { enabled: true, connected: true, status: "Connected" },
};

const disconnectedAppState: Pick<AppState, "hevy"> = {
  hevy: { enabled: false, connected: false, status: "Not connected" },
};

const queueState: HevyQueueState = {
  enabled: true,
  counts: { inFlight: 2, problems: 1, skipped: 1 },
  inFlight: [
    {
      hevyId: "hevy-leg-day",
      title: "Leg Day",
      status: "syncing",
      error: null,
      startDisplay: "Jul 20, 06:12",
      needsMapping: false,
      hasOpenOperation: true,
      resyncable: false,
    },
    {
      hevyId: "hevy-pull-day",
      title: "Pull Day",
      status: "waiting_watch",
      error: null,
      startDisplay: "Jul 20, 07:40",
      needsMapping: false,
      hasOpenOperation: false,
      resyncable: false,
    },
  ],
  problems: [
    {
      hevyId: "hevy-upper-body",
      title: "Upper Body",
      status: "needs_mapping",
      error: "2 exercises need mapping before sync",
      startDisplay: "Jul 19, 18:04",
      needsMapping: true,
      hasOpenOperation: false,
      resyncable: false,
    },
  ],
  skipped: [
    {
      hevyId: "hevy-mobility-flow",
      title: "Mobility Flow",
      status: "skipped",
      error: null,
      startDisplay: "Jul 18, 09:00",
      needsMapping: false,
      hasOpenOperation: false,
      resyncable: false,
    },
  ],
};

const emptyQueueState: HevyQueueState = {
  enabled: true,
  counts: { inFlight: 0, problems: 0, skipped: 0 },
  inFlight: [],
  problems: [],
  skipped: [],
};

const toolsState: HevyToolsState = {
  mappings: [
    {
      templateId: "tmpl-unmapped",
      title: "Bulgarian Split Squat",
      isCustom: false,
      muscleGroup: "Legs",
      mapped: false,
      unmapped: true,
      garminRejected: false,
      suggested: false,
      source: "",
      category: null,
      subcategory: null,
      categoryName: null,
      subcategoryName: null,
    },
    {
      templateId: "tmpl-mapped",
      title: "Barbell Bench Press",
      isCustom: false,
      muscleGroup: "Chest",
      mapped: true,
      unmapped: false,
      garminRejected: false,
      suggested: false,
      source: "automatic",
      category: 4,
      subcategory: 0,
      categoryName: "Bench Press",
      subcategoryName: "Barbell",
    },
  ],
  categories: [],
};

const emptyToolsState: HevyToolsState = { mappings: [], categories: [] };

function renderHevy({
  appState = connectedAppState,
  queue = queueState,
  tools = toolsState,
}: {
  appState?: Pick<AppState, "hevy">;
  queue?: HevyQueueState;
  tools?: HevyToolsState;
} = {}) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <HevyView appState={appState} queue={queue} tools={tools} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

test("hub shows in-flight and problem counts", async () => {
  renderHevy();
  expect(await screen.findByText(/2 in flight/i)).toBeInTheDocument();
  expect(screen.getByText(/1 problem/i)).toBeInTheDocument();
});

test("mapping summary links to the editor for an unmapped exercise", async () => {
  renderHevy();
  expect(await screen.findByRole("link", { name: /map/i })).toHaveAttribute(
    "href",
    "/hevy/mapping/tmpl-unmapped",
  );
});

test("mapping summary omits exercises that are already mapped", () => {
  renderHevy();
  expect(screen.queryByText("Barbell Bench Press")).not.toBeInTheDocument();
});

// Backfill is inline on the hub now — the card that held nothing but a link
// to an overlay was one surface too many for a single task.
test("backfill is inline on the hub, not a link to somewhere else", () => {
  renderHevy();
  expect(screen.queryByRole("link", { name: /preview backfill/i })).toBeNull();
  expect(screen.getByLabelText("Since")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Preview" })).toBeInTheDocument();
});

test("presents skipped queue items inline with an unskip action, not hidden", () => {
  renderHevy();
  expect(screen.getByText("Mobility Flow")).toBeInTheDocument();
  expect(screen.getByText(/1 skipped/i)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Unskip" })).toBeInTheDocument();
});

test("shows an empty state pointing at Settings when Hevy is not connected", () => {
  renderHevy({ appState: disconnectedAppState });
  expect(screen.getByText(/connect hevy/i)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /go to settings/i })).toHaveAttribute(
    "href",
    "/settings#connections",
  );
  expect(screen.queryByText("Sync queue")).not.toBeInTheDocument();
});

test("still renders the three cards when connected but the queue and mappings are empty", () => {
  renderHevy({ queue: emptyQueueState, tools: emptyToolsState });
  expect(screen.getByText("Sync queue")).toBeInTheDocument();
  expect(screen.getByText(/nothing in the queue/i)).toBeInTheDocument();
  expect(screen.getByText("Exercise mapping")).toBeInTheDocument();
  expect(screen.getByText("Backfill older workouts")).toBeInTheDocument();
});
