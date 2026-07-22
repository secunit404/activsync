import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, Outlet, RouterProvider } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { BackfillResult } from "@/lib/api";
import HevyBackfill from "./hevy-backfill";
import HevyMapping from "./hevy-mapping";

beforeEach(() => {
  vi.clearAllMocks();
});

const { previewHevyBackfill, runHevyBackfill, getHevyTools } = vi.hoisted(() => ({
  previewHevyBackfill: vi.fn(),
  runHevyBackfill: vi.fn(),
  getHevyTools: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, previewHevyBackfill, runHevyBackfill, getHevyTools };
});

const { toastSuccess, toastError } = vi.hoisted(() => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: { success: toastSuccess, error: toastError },
}));

type Item = BackfillResult["items"][number];

function item(overrides: Partial<Item>): Item {
  return {
    hevyId: "hw-1",
    title: "Leg Session",
    startTime: "2026-06-03T10:00:00+00:00",
    action: "replace",
    twinActivityId: null,
    missingTemplateIds: [],
    ...overrides,
  };
}

// 3 importable (1 LINK, 2 CREATE) + 1 locked — matches the task brief's
// "3 importable, 1 locked" fixture shape exactly.
function fourItemResult(overrides: Partial<BackfillResult> = {}): BackfillResult {
  return {
    message: "4 workouts found. Nothing has been written yet.",
    since: "2026-06-01",
    ran: false,
    linked: 0,
    items: [
      item({
        hevyId: "hw-link",
        title: "Full Body",
        action: "linked_existing",
        twinActivityId: 910001,
      }),
      item({ hevyId: "hw-create-1", title: "Push A", action: "replace" }),
      item({ hevyId: "hw-create-2", title: "Legs B", action: "passive" }),
      item({
        hevyId: "hw-locked",
        title: "Unmapped workout",
        action: "needs_mapping",
        missingTemplateIds: ["tmpl-unmapped"],
      }),
    ],
    ...overrides,
  };
}

function renderBackfill() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });

  const router = createMemoryRouter(
    [
      {
        path: "/hevy",
        element: <Outlet />,
        children: [
          { index: true, element: <div>Hub</div> },
          {
            path: "backfill",
            element: <HevyBackfill />,
            // Mirrors routes.ts: the editor is nested, so mounting it leaves
            // HevyBackfill mounted and its preview state intact.
            children: [{ path: "mapping/:templateId", element: <HevyMapping /> }],
          },
        ],
      },
    ],
    { initialEntries: ["/hevy/backfill"] },
  );

  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );

  return { router, client };
}

async function preview() {
  await userEvent.click(screen.getByRole("button", { name: "Preview" }));
}

test("locked rows are not selectable", async () => {
  previewHevyBackfill.mockResolvedValue(fourItemResult());
  renderBackfill();
  await preview();

  const locked = await screen.findByRole("checkbox", { name: /Unmapped workout/ });
  expect(locked).toBeDisabled();
});

test("select all importable ignores locked rows", async () => {
  previewHevyBackfill.mockResolvedValue(fourItemResult());
  renderBackfill();
  await preview();

  await userEvent.click(await screen.findByRole("checkbox", { name: /select all importable/i }));
  expect(screen.getByRole("button", { name: /import 3 selected/i })).toBeEnabled();
});

// The editor is a CHILD of backfill, not a sibling — see routes.ts.
test("locked row links to the nested mapping editor for its missing template", async () => {
  previewHevyBackfill.mockResolvedValue(fourItemResult());
  renderBackfill();
  await preview();

  expect(await screen.findByRole("link", { name: /map/i })).toHaveAttribute(
    "href",
    "/hevy/backfill/mapping/tmpl-unmapped",
  );
});

test("a locked row with no missing template id has no map link, but stays locked", async () => {
  previewHevyBackfill.mockResolvedValue(
    fourItemResult({
      items: [
        item({
          hevyId: "hw-unmappable",
          title: "Mystery session",
          action: "needs_mapping",
          missingTemplateIds: [],
        }),
      ],
    }),
  );
  renderBackfill();
  await preview();

  expect(await screen.findByRole("checkbox", { name: /Mystery session/i })).toBeDisabled();
  // Same control as the linkable case, disabled — not a second badge.
  expect(screen.queryByRole("link", { name: /map/i })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: /map/i })).toBeDisabled();
});

test("import button count and confirmation reflect only what's checked, not every scanned row", async () => {
  previewHevyBackfill.mockResolvedValue(fourItemResult());
  renderBackfill();
  await preview();

  const linkRow = await screen.findByRole("checkbox", { name: "Select Full Body" });
  await userEvent.click(linkRow);
  expect(screen.getByRole("button", { name: "Import 1 selected" })).toBeInTheDocument();
});

test("a fresh preview resets the selection", async () => {
  previewHevyBackfill.mockResolvedValueOnce(fourItemResult());
  renderBackfill();
  await preview();

  await userEvent.click(await screen.findByRole("checkbox", { name: /select all importable/i }));
  expect(screen.getByRole("button", { name: "Import 3 selected" })).toBeInTheDocument();

  previewHevyBackfill.mockResolvedValueOnce(fourItemResult());
  await preview();

  expect(await screen.findByRole("button", { name: "Import 0 selected" })).toBeInTheDocument();
});

test("importing calls runHevyBackfill with since and reports the result via toast", async () => {
  previewHevyBackfill.mockResolvedValue(fourItemResult());
  runHevyBackfill.mockResolvedValue({
    message: "Backfill complete: 4 workouts ingested, 1 linked to existing Garmin activities.",
    since: "2026-06-01",
    ran: true,
    linked: 1,
    items: [],
  });
  renderBackfill();
  fireEvent.change(screen.getByLabelText("Since"), { target: { value: "2026-06-01" } });
  await preview();

  await userEvent.click(await screen.findByRole("checkbox", { name: /select all importable/i }));
  await userEvent.click(screen.getByRole("button", { name: "Import 3 selected" }));

  await userEvent.click(await screen.findByRole("button", { name: "Import" }));

  await waitFor(() =>
    expect(runHevyBackfill).toHaveBeenCalledWith("2026-06-01", [
      "hw-link",
      "hw-create-1",
      "hw-create-2",
    ]),
  );
  await waitFor(() =>
    expect(toastSuccess).toHaveBeenCalledWith(
      "Backfill complete: 4 workouts ingested, 1 linked to existing Garmin activities.",
    ),
  );
});

test("deselecting a row before importing excludes it from the sent ids, and locked rows never reach the selection", async () => {
  previewHevyBackfill.mockResolvedValue(fourItemResult());
  runHevyBackfill.mockResolvedValue({
    message: "Backfill complete: 2 workouts ingested, 1 linked to existing Garmin activities.",
    since: "2026-06-01",
    ran: true,
    linked: 1,
    items: [],
  });
  renderBackfill();
  fireEvent.change(screen.getByLabelText("Since"), { target: { value: "2026-06-01" } });
  await preview();

  // Select all 3 importable rows, then untick one — the locked row was
  // never selectable to begin with, so this exercises both guardrails: a
  // deliberate uncheck, and a checkbox that was disabled from the start.
  await userEvent.click(await screen.findByRole("checkbox", { name: /select all importable/i }));
  await userEvent.click(screen.getByRole("checkbox", { name: "Select Push A" }));
  await userEvent.click(screen.getByRole("button", { name: "Import 2 selected" }));
  await userEvent.click(await screen.findByRole("button", { name: "Import" }));

  await waitFor(() =>
    expect(runHevyBackfill).toHaveBeenCalledWith("2026-06-01", ["hw-link", "hw-create-2"]),
  );
  const [, sentIds] = runHevyBackfill.mock.calls[0];
  expect(sentIds).not.toContain("hw-create-1");
  expect(sentIds).not.toContain("hw-locked");
});

test("a preview failure reports an error toast", async () => {
  previewHevyBackfill.mockRejectedValue(new Error("Hevy rejected the stored API key."));
  renderBackfill();
  await preview();

  await waitFor(() =>
    expect(toastError).toHaveBeenCalledWith(
      "Hevy rejected the stored API key.",
      expect.objectContaining({ duration: expect.any(Number) }),
    ),
  );
});

test("the Import footer is absent until a preview has run", () => {
  renderBackfill();
  expect(screen.queryByRole("button", { name: /import \d+ selected/i })).not.toBeInTheDocument();
});

// The whole point of nesting the editor under backfill: opening it leaves
// this overlay mounted, so the preview result and the ticked selection
// survive. As a sibling route it unmounted them, and Cancel returned the
// user to an empty form.
test("mapping a locked row keeps the backfill preview and selection alive", async () => {
  previewHevyBackfill.mockResolvedValue(fourItemResult());
  getHevyTools.mockResolvedValue({
    mappings: [
      {
        templateId: "tmpl-unmapped",
        title: "Landmine Press",
        isCustom: true,
        muscleGroup: "shoulders",
        mapped: false,
        unmapped: true,
        garminRejected: false,
        suggested: false,
        category: null,
        subcategory: null,
        categoryName: null,
        subcategoryName: null,
      },
    ],
    categories: [
      {
        value: 3,
        label: "Strength",
        subcategories: [{ value: 7, label: "Shoulder Press" }],
      },
    ],
  });
  renderBackfill();
  await preview();

  await userEvent.click(await screen.findByRole("checkbox", { name: "Select Full Body" }));
  expect(screen.getByRole("button", { name: "Import 1 selected" })).toBeInTheDocument();

  await userEvent.click(screen.getByRole("link", { name: /map/i }));

  // Editor is open, and the backfill screen is still rendered behind it.
  expect(await screen.findByText("Landmine Press")).toBeVisible();
  expect(screen.getByText("Unmapped workout")).toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

  // Back on backfill with the scan AND the tick still there — not a reset.
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Import 1 selected" })).toBeInTheDocument(),
  );
});
