import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { HevyToolsState } from "@/lib/api";
import HevyMappings from "./hevy-mappings";

const { getHevyTools } = vi.hoisted(() => ({ getHevyTools: vi.fn() }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, getHevyTools };
});

type Mapping = HevyToolsState["mappings"][number];

function mapping(overrides: Partial<Mapping> = {}): Mapping {
  return {
    templateId: "tpl",
    title: "Exercise",
    isCustom: false,
    muscleGroup: "",
    mapped: true,
    unmapped: false,
    garminRejected: false,
    suggested: false,
    source: "automatic",
    category: 1,
    subcategory: 2,
    categoryName: "Strength",
    subcategoryName: "Press",
    ...overrides,
  };
}

const toolsState: HevyToolsState = {
  mappings: [
    mapping({ templateId: "tpl-1", title: "Landmine Press" }),
    mapping({
      templateId: "tpl-2",
      title: "Ring Dips",
      mapped: false,
      unmapped: true,
      source: "",
      categoryName: null,
      subcategoryName: null,
    }),
    mapping({ templateId: "tpl-3", title: "Sled Push" }),
  ],
  categories: [],
};

beforeEach(() => {
  vi.clearAllMocks();
  getHevyTools.mockResolvedValue(toolsState);
});

function renderMappings() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <HevyMappings />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

// The gap this page fills: the hub only ever listed exercises still needing
// action, so an already-mapped exercise could not be reviewed or changed.
test("lists every mapping, mapped and unmapped alike", async () => {
  renderMappings();
  expect(await screen.findByText("Landmine Press")).toBeVisible();
  expect(screen.getByText("Ring Dips")).toBeVisible();
  expect(screen.getByText("Sled Push")).toBeVisible();
});

test("the needs-mapping filter hides already-mapped exercises", async () => {
  renderMappings();
  await screen.findByText("Landmine Press");
  await userEvent.click(screen.getByRole("button", { name: "Needs mapping" }));
  expect(screen.getByText("Ring Dips")).toBeVisible();
  expect(screen.queryByText("Landmine Press")).toBeNull();
});

test("the mapped filter hides exercises still needing action", async () => {
  renderMappings();
  await screen.findByText("Landmine Press");
  await userEvent.click(screen.getByRole("button", { name: "Mapped" }));
  expect(screen.getByText("Landmine Press")).toBeVisible();
  expect(screen.queryByText("Ring Dips")).toBeNull();
});

test("search narrows by title, case-insensitively", async () => {
  renderMappings();
  await screen.findByText("Landmine Press");
  await userEvent.type(screen.getByLabelText("Search exercises"), "sled");
  expect(screen.getByText("Sled Push")).toBeVisible();
  expect(screen.queryByText("Landmine Press")).toBeNull();
});

test("search and filter compose rather than override each other", async () => {
  renderMappings();
  await screen.findByText("Landmine Press");
  await userEvent.click(screen.getByRole("button", { name: "Mapped" }));
  await userEvent.type(screen.getByLabelText("Search exercises"), "ring");
  // Ring Dips matches the search but is unmapped, so the filter excludes it.
  expect(screen.queryByText("Ring Dips")).toBeNull();
  expect(screen.getByText(/No exercises match/)).toBeVisible();
});

test("an empty result explains itself", async () => {
  renderMappings();
  await screen.findByText("Landmine Press");
  await userEvent.type(screen.getByLabelText("Search exercises"), "zzz");
  expect(screen.getByText(/No exercises match/)).toBeVisible();
});

test("an empty mapping list does not look like a failed load", async () => {
  getHevyTools.mockResolvedValue({ mappings: [], categories: [] });
  renderMappings();
  expect(await screen.findByText(/No exercises match/)).toBeVisible();
});
