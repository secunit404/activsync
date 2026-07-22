import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, expect, test, vi } from "vitest";

import type { SettingsState } from "@/lib/api";
import { hevyDraftFromState, type HevyDraft } from "@/lib/hevy-draft";
import { HevySettings } from "./settings-hevy";

const { getHevyDeviceOptions } = vi.hoisted(() => ({
  getHevyDeviceOptions: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, getHevyDeviceOptions };
});

const hevyState: SettingsState["hevy"] = {
  connected: true,
  status: "Connected",
  apiKeySaved: true,
  enabled: true,
  watchStrategy: "replace",
  matchMode: "automatic",
  descriptionTemplate: "{title}\n{exercises}",
  summaryOnStructured: true,
  graceMinutes: 120,
  pollIntervalMinutes: 10,
  identity: { manufacturer: null, product: null, serial: null },
  identityDisplay: "not yet detected",
  profileOverride: { weightKg: null, birthYear: null, vo2max: null, sex: null },
};

const state = { hevy: hevyState } as SettingsState;

beforeEach(() => {
  vi.clearAllMocks();
  getHevyDeviceOptions.mockResolvedValue({
    // Garmin is the only manufacturer the endpoint serves — ActivSync writes
    // Garmin FIT files and nothing else. See the device-options route.
    manufacturers: [{ value: 1, label: "Garmin" }],
    products: [
      { value: 2050, label: "Fenix3" },
      { value: 1482, label: "Fr10" },
    ],
  });
});

function renderHevySettings(draft: HevyDraft = hevyDraftFromState(hevyState)) {
  const onChange = vi.fn();
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <HevySettings state={state} draft={draft} onChange={onChange} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { onChange };
}

test("lets the user switch between reviewed and automatic matching", async () => {
  const { onChange } = renderHevySettings(
    hevyDraftFromState({ ...hevyState, matchMode: "review" }),
  );

  expect(screen.getByRole("radio", { name: "Review each match" })).toBeChecked();
  await userEvent.click(screen.getByRole("radio", { name: "Automatic" }));

  expect(onChange).toHaveBeenLastCalledWith(
    expect.objectContaining({ matchMode: "automatic" }),
  );
});

test("edits the description template and structured-summary preference", async () => {
  const { onChange } = renderHevySettings();

  expect(screen.getByLabelText("Write summary for Merge and Replace")).toBeChecked();
  await userEvent.click(
    screen.getByLabelText("Write summary for Merge and Replace"),
  );
  expect(onChange).toHaveBeenLastCalledWith(
    expect.objectContaining({ summaryOnStructured: false }),
  );

  fireEvent.change(screen.getByLabelText("Description template"), {
    target: { value: "{title} — custom" },
  });
  expect(onChange).toHaveBeenLastCalledWith(
    expect.objectContaining({ descriptionTemplate: "{title} — custom" }),
  );
});

test("shows a live plain-text sample beside the description template", () => {
  renderHevySettings(
    hevyDraftFromState({
      ...hevyState,
      descriptionTemplate: "**{title}**\n\n{exercises}",
    }),
  );

  const preview = screen.getByRole("region", { name: "Plain-text preview" });
  expect(preview).toHaveTextContent("**Afternoon workout 💪**");
  expect(preview).toHaveTextContent("Chest Fly (Machine)");
});

test("documents and previews the emoji-free clean title placeholder", () => {
  renderHevySettings(
    hevyDraftFromState({
      ...hevyState,
      descriptionTemplate: "{clean_title}",
    }),
  );

  expect(screen.getByText(/clean_title.*removes emoji/i)).toBeVisible();
  expect(screen.getByRole("region", { name: "Plain-text preview" })).toHaveTextContent(
    "Afternoon workout",
  );
});

async function openOverrides() {
  await userEvent.click(screen.getByText("Device and profile overrides"));
}

test("manufacturer and product are selects, serial stays a text entry", async () => {
  renderHevySettings();
  await openOverrides();

  expect(await screen.findByLabelText("Manufacturer")).toHaveProperty("tagName", "SELECT");
  expect(screen.getByLabelText("Product")).toHaveProperty("tagName", "SELECT");
  // A serial is per-device and unguessable — there is no list to pick from.
  expect(screen.getByLabelText("Serial")).toHaveProperty("tagName", "INPUT");
});

test("each select offers Automatic plus the served options", async () => {
  renderHevySettings();
  await openOverrides();

  const manufacturer = await screen.findByLabelText("Manufacturer");
  expect(within(manufacturer).getByRole("option", { name: "Automatic" })).toBeInTheDocument();
  expect(within(manufacturer).getByRole("option", { name: "Garmin" })).toBeInTheDocument();
  // Automatic + Garmin and nothing else: the endpoint serves one
  // manufacturer, so the picker must not imply others would work.
  expect(within(manufacturer).getAllByRole("option")).toHaveLength(2);
  expect(
    within(screen.getByLabelText("Product")).getByRole("option", { name: "Fenix3" }),
  ).toBeInTheDocument();
});

test("choosing a manufacturer reports the numeric value up as a string", async () => {
  const { onChange } = renderHevySettings();
  await openOverrides();
  await userEvent.selectOptions(await screen.findByLabelText("Manufacturer"), "1");

  // hevyDraftToPayload parses these back to numbers; the draft itself is
  // string-backed so the controlled selects can hold "" for "no override".
  expect(onChange).toHaveBeenLastCalledWith(
    expect.objectContaining({
      identity: expect.objectContaining({ manufacturer: "1" }),
    }),
  );
});

test("a saved identity is reflected as the selected option", async () => {
  renderHevySettings(
    hevyDraftFromState({
      ...hevyState,
      identity: { manufacturer: 1, product: 2050, serial: 987 },
    }),
  );
  await openOverrides();

  expect(await screen.findByLabelText("Manufacturer")).toHaveValue("1");
  expect(screen.getByLabelText("Product")).toHaveValue("2050");
  expect(screen.getByLabelText("Serial")).toHaveValue(987);
});

// The endpoint is reference data; if it has not resolved yet the selects
// must still render with their Automatic option rather than crash or show
// an empty control.
test("the selects render before the options have loaded", async () => {
  getHevyDeviceOptions.mockReturnValue(new Promise(() => {}));
  renderHevySettings();
  await openOverrides();

  const manufacturer = screen.getByLabelText("Manufacturer");
  expect(within(manufacturer).getByRole("option", { name: "Automatic" })).toBeInTheDocument();
});
