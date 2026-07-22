import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, Link, Outlet, RouterProvider } from "react-router";
import { expect, test, vi } from "vitest";

import type { HevyToolsState } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import HevyMapping from "./hevy-mapping";

const { getHevyTools, saveExerciseMapping } = vi.hoisted(() => ({
  getHevyTools: vi.fn(),
  saveExerciseMapping: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, getHevyTools, saveExerciseMapping };
});

type Mapping = HevyToolsState["mappings"][number];

function mapping(overrides: Partial<Mapping>): Mapping {
  return {
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
    ...overrides,
  };
}

// Real value/label shape from `HevyToolsState.categories` — two categories,
// each with its own subcategory set, so a category switch has somewhere
// stale to leave behind. `CYCLING` matches the real API shape for one of
// the nine Garmin categories with zero subcategories (id 33, verified
// against `SUBCATEGORY_NAMES` in `hevy_mapper.py`) — a dead end the picker
// must not offer, since `save_mapping` can never validate a subcategory for
// it (`SUBCATEGORY_NAMES.get(33, {})` is always `{}`).
const categories: HevyToolsState["categories"] = [
  {
    value: 4,
    label: "Bench Press",
    subcategories: [
      { value: 0, label: "Barbell" },
      { value: 1, label: "Dumbbell" },
    ],
  },
  {
    value: 12,
    label: "Squat",
    subcategories: [
      { value: 120, label: "Back Squat" },
      { value: 121, label: "Front Squat" },
    ],
  },
  {
    value: 18,
    label: "Deadlift",
    subcategories: [{ value: 180, label: "Conventional" }],
  },
  {
    value: 33,
    label: "Cycling",
    subcategories: [],
  },
];

const toolsState: HevyToolsState = {
  mappings: [
    mapping({}),
    mapping({
      templateId: "tmpl-mapped",
      title: "Barbell Bench Press",
      mapped: true,
      unmapped: false,
      source: "user",
      category: 4,
      subcategory: 0,
      categoryName: "Bench Press",
      subcategoryName: "Barbell",
    }),
    // The common case now that the list shows everything: resolved by the
    // ported tables, nobody chose it.
    mapping({
      templateId: "tmpl-automatic",
      title: "Back Squat",
      mapped: true,
      unmapped: false,
      source: "automatic",
      category: 12,
      subcategory: 120,
      categoryName: "Squat",
      subcategoryName: "Back Squat",
    }),
    mapping({
      templateId: "tmpl-user",
      title: "Front Squat",
      mapped: true,
      unmapped: false,
      source: "user",
      category: 12,
      subcategory: 121,
      categoryName: "Squat",
      subcategoryName: "Front Squat",
    }),
    mapping({
      templateId: "tmpl-rejected",
      title: "Ring Row",
      mapped: false,
      unmapped: false,
      garminRejected: true,
      category: 4,
      subcategory: 1,
      categoryName: "Bench Press",
      subcategoryName: "Dumbbell",
    }),
  ],
  categories,
};

getHevyTools.mockResolvedValue(toolsState);

function renderMapping(templateId: string, state: HevyToolsState = toolsState) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  client.setQueryData(queryKeys.hevyTools, state);

  const router = createMemoryRouter(
    [
      {
        path: "/hevy",
        element: <Outlet />,
        children: [
          { index: true, element: <div>Hub</div> },
          { path: "mapping/:templateId", element: <HevyMapping /> },
        ],
      },
    ],
    { initialEntries: [`/hevy/mapping/${templateId}`] },
  );

  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );

  return { router, client };
}

test("changing category resets the subcategory choice", async () => {
  renderMapping("tmpl-unmapped");
  await userEvent.selectOptions(await screen.findByLabelText("Category"), "12");
  await userEvent.selectOptions(screen.getByLabelText("Subcategory"), "120");
  await userEvent.selectOptions(screen.getByLabelText("Category"), "18");
  expect(screen.getByLabelText("Subcategory")).toHaveValue("");
});

test("preview shows the resulting mapping", async () => {
  renderMapping("tmpl-unmapped");
  await userEvent.selectOptions(await screen.findByLabelText("Category"), "12");
  await userEvent.selectOptions(screen.getByLabelText("Subcategory"), "120");
  expect(screen.getByTestId("syncs-as")).toHaveTextContent("Squat › Back Squat");
});

test("save is disabled until both levels are chosen", async () => {
  renderMapping("tmpl-unmapped");
  expect(await screen.findByRole("button", { name: "Save mapping" })).toBeDisabled();
});

test("subcategory select is disabled until a category is chosen", async () => {
  renderMapping("tmpl-unmapped");
  expect(await screen.findByLabelText("Subcategory")).toBeDisabled();
});

test("a mapped exercise pre-fills both selects and enables Save", async () => {
  renderMapping("tmpl-mapped");
  expect(await screen.findByLabelText("Category")).toHaveValue("4");
  expect(screen.getByLabelText("Subcategory")).toHaveValue("0");
  expect(screen.getByRole("button", { name: "Save mapping" })).not.toBeDisabled();
});

test("a Garmin-rejected exercise explains why it needs a different pick", async () => {
  renderMapping("tmpl-rejected");
  expect(
    await screen.findByText(/Garmin rejected the previous mapping/i),
  ).toBeInTheDocument();
});

// The editor is reachable for every exercise now, not just the ones nothing
// resolves, so the banner has to name which of the three states you are in
// rather than always claiming no default exists.
test("an unresolved exercise says nothing maps it yet", async () => {
  renderMapping("tmpl-unmapped");
  expect(await screen.findByText(/Nothing maps this exercise yet/i)).toBeInTheDocument();
});

test("an automatically mapped exercise says saving replaces the automatic pair", async () => {
  renderMapping("tmpl-automatic");
  expect(
    await screen.findByText(/maps this automatically. Saving here replaces that/i),
  ).toBeInTheDocument();
});

test("a user-set exercise says saving replaces their own choice", async () => {
  renderMapping("tmpl-user");
  expect(await screen.findByText(/You set this mapping/i)).toBeInTheDocument();
});

test("an unknown templateId shows a real message, not a blank overlay", async () => {
  renderMapping("tmpl-nonexistent");
  expect(await screen.findByRole("dialog")).toBeInTheDocument();
  expect(screen.getByText(/isn't in the current mapping list/i)).toBeInTheDocument();
});

test("Cancel navigates back to the hub without saving", async () => {
  const { router } = renderMapping("tmpl-unmapped");
  await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  expect(saveExerciseMapping).not.toHaveBeenCalled();
  await waitFor(() => expect(router.state.location.pathname).toBe("/hevy"));
});

test("Cancel returns to backfill when the editor is mounted underneath it", async () => {
  // This module is mounted at two paths (see routes.ts): under `/hevy` from
  // the hub, and under `/hevy/backfill` from a locked row. `close()` is a
  // plain `navigate("..")`, so the return target is structural — the parent
  // route IS the screen the user came from. This covers the nested mount;
  // the test above covers the hub mount.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  client.setQueryData(queryKeys.hevyTools, toolsState);

  const router = createMemoryRouter(
    [
      {
        path: "/hevy",
        element: <Outlet />,
        children: [
          {
            path: "backfill",
            element: (
              <>
                <Link to="/hevy/backfill/mapping/tmpl-unmapped">Map →</Link>
                <Outlet />
              </>
            ),
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

  await userEvent.click(screen.getByRole("link", { name: "Map →" }));
  await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/hevy/backfill"));
});

test("Escape navigates back to the hub", async () => {
  const { router } = renderMapping("tmpl-unmapped");
  await screen.findByRole("dialog");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(router.state.location.pathname).toBe("/hevy"));
});

test("saving calls saveExerciseMapping with the chosen ids and returns to the hub", async () => {
  saveExerciseMapping.mockResolvedValue({ message: "Exercise mapping saved." });
  const { router } = renderMapping("tmpl-unmapped");

  await userEvent.selectOptions(await screen.findByLabelText("Category"), "12");
  await userEvent.selectOptions(screen.getByLabelText("Subcategory"), "120");
  await userEvent.click(screen.getByRole("button", { name: "Save mapping" }));

  await waitFor(() =>
    expect(saveExerciseMapping).toHaveBeenCalledWith("tmpl-unmapped", 12, 120),
  );
  await waitFor(() => expect(router.state.location.pathname).toBe("/hevy"));
});

test("a category with no subcategories is not offered — it would be a dead end Save can never clear", async () => {
  renderMapping("tmpl-unmapped");
  const select = await screen.findByLabelText("Category");
  const optionLabels = within(select)
    .getAllByRole("option")
    .map((option) => option.textContent);
  expect(optionLabels).not.toContain("Cycling");
  // Sanity check the fixture actually contains a zero-subcategory category,
  // so this test would fail for the right reason if the filter regresses.
  expect(categories.find((option) => option.label === "Cycling")?.subcategories).toEqual([]);
});

test("an existing mapping pointing at a since-filtered, subcategory-less category still displays instead of vanishing", async () => {
  // Unreachable via `save_mapping` today (it rejects any subcategory for a
  // category with an empty `SUBCATEGORY_NAMES` entry), but the picker must
  // not silently drop a saved value if the taxonomy ever changes under it.
  const staleState: HevyToolsState = {
    ...toolsState,
    mappings: [
      ...toolsState.mappings,
      mapping({
        templateId: "tmpl-stale-category",
        title: "Stationary Bike Sprint",
        mapped: true,
        unmapped: false,
        category: 33,
        subcategory: 0,
        categoryName: "Cycling",
        subcategoryName: null,
      }),
    ],
  };

  renderMapping("tmpl-stale-category", staleState);

  const select = await screen.findByLabelText("Category");
  expect(select).toHaveValue("33");
  expect(within(select).getByRole("option", { name: "Cycling" })).toBeInTheDocument();
});
